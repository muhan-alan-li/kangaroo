#!/usr/bin/env node
// One page through a real, local Chromium, on its way back to Python as JSON.
//
// This exists because `urllib` cannot run JavaScript: a team page rendered client-side, or an
// address injected by a script into a `mailto:` link, is invisible to a plain GET. Playwright and
// its Chromium are already on this machine (see README in this directory), so this is a local
// tool with no vendor and no API key.
//
//     node server/brief/browser_fetch.mjs <url> [--wait ms] [--timeout ms]
//
// Prints {url, final_url, status, title, text, emails} on stdout. Errors go to stderr as JSON and
// exit non-zero. Images, media and fonts are aborted to keep a fetch to a couple of seconds; CSS
// is kept, because `innerText` depends on it.

import { createRequire } from "node:module";
import { execSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);

const UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) " +
  "Chrome/124.0 Safari/537.36";

const EMAIL_RE = /[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}/g;
const EMAIL_EXACT = /^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$/;
const EMAIL_JUNK =
  /(no-?reply|donotreply|do-not-reply|example\.(com|org)|sentry|wixpress|users\.noreply\.github\.com|\.png|\.jpe?g|\.gif|\.svg|\.webp|\.css|\.js$|@2x)/i;

// Playwright lives wherever it was installed. Try, in order: an explicit override, every npm
// `npx` cache, the npm global root, and this file's ancestors (a workspace install).
function candidateBases() {
  const bases = [];
  if (process.env.PLAYWRIGHT_BASE) bases.push(process.env.PLAYWRIGHT_BASE);
  const npxRoot = path.join(os.homedir(), ".npm", "_npx");
  try {
    for (const entry of fs.readdirSync(npxRoot)) bases.push(path.join(npxRoot, entry));
  } catch {
    /* no npx cache */
  }
  try {
    const globalRoot = execSync("npm root -g", {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    if (globalRoot) bases.push(path.dirname(globalRoot));
  } catch {
    /* npm not on PATH */
  }
  let dir = path.dirname(fileURLToPath(import.meta.url));
  for (;;) {
    bases.push(dir);
    const up = path.dirname(dir);
    if (up === dir) break;
    dir = up;
  }
  return bases;
}

function loadPlaywright() {
  for (const base of candidateBases()) {
    try {
      return require(require.resolve("playwright", { paths: [base] }));
    } catch {
      /* try the next base */
    }
  }
  throw new Error(
    "playwright not found. Install it (npm i -g playwright) or set PLAYWRIGHT_BASE to a " +
      "directory whose node_modules contains it."
  );
}

function extractEmails(mailtos, text, html) {
  const out = [];
  const seen = new Set();
  const push = (raw) => {
    const email = String(raw || "")
      .replace(/^mailto:/i, "")
      .split("?")[0]
      .trim()
      .replace(/^[<(`'"]+|[>`)'".,;:]+$/g, "");
    if (!EMAIL_EXACT.test(email) || EMAIL_JUNK.test(email)) return;
    const key = email.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      out.push(email);
    }
  };
  mailtos.forEach(push);
  for (const match of text.match(EMAIL_RE) || []) push(match);
  for (const match of html.match(EMAIL_RE) || []) push(match);
  return out;
}

function option(args, name, fallback) {
  const index = args.indexOf(name);
  return index >= 0 && args[index + 1] !== undefined ? Number(args[index + 1]) : fallback;
}

const args = process.argv.slice(2);
const url = args[0];
if (!url || url.startsWith("--")) {
  process.stderr.write(JSON.stringify({ error: "usage: browser_fetch.mjs <url> [--wait ms] [--timeout ms]" }));
  process.exit(2);
}
const navTimeout = option(args, "--timeout", 45000);
const settle = option(args, "--wait", 1200);

const { chromium } = loadPlaywright();
const browser = await chromium.launch({
  headless: true,
  args: ["--no-sandbox", "--disable-dev-shm-usage"],
});
try {
  const context = await browser.newContext({
    userAgent: UA,
    viewport: { width: 1366, height: 900 },
    locale: "en-CA",
  });
  const page = await context.newPage();
  await page.route("**/*", (route) => {
    const type = route.request().resourceType();
    if (type === "image" || type === "media" || type === "font") return route.abort();
    return route.continue();
  });
  const response = await page.goto(url, { waitUntil: "domcontentloaded", timeout: navTimeout });
  await page.waitForLoadState("load", { timeout: 8000 }).catch(() => {});
  await page.waitForTimeout(settle);
  const dom = await page.evaluate(() => ({
    mailtos: Array.from(document.querySelectorAll('a[href^="mailto:" i]')).map((a) =>
      a.getAttribute("href")
    ),
    text: (document.body && document.body.innerText) || "",
    html: (document.documentElement && document.documentElement.outerHTML) || "",
  }));
  const result = {
    url,
    final_url: page.url(),
    status: response ? response.status() : null,
    title: await page.title(),
    text: dom.text.replace(/\s+/g, " ").trim().slice(0, 8000),
    emails: extractEmails(dom.mailtos, dom.text, dom.html).slice(0, 20),
  };
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  process.stderr.write(JSON.stringify({ error: String((error && error.message) || error) }));
  process.exit(1);
} finally {
  await browser.close();
}
