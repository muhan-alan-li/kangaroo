"""Keywords, skills and outreach addresses read out of a posting's own text.

Two jobs, both of them about tailoring rather than filtering:

1. `extract_keywords` answers "what should the resume say for this posting?". It matches a
   curated vocabulary of skills and technologies against the posting text, then splits the
   hits into the ones the candidate's profile already claims (`have`) and the ones it does
   not (`gap`). The gap list is the tailoring prompt: a term the employer asks for that the
   resume never mentions is either work the candidate has not written down yet, or a real
   gap worth knowing about before applying. It is never a reason to drop a posting.

2. `extract_emails` pulls an outreach address out of the posting text, when the employer
   published one in it. Only addresses literally present in the posting are returned --
   nothing is ever patterned or guessed from a company's domain. A guessed address is a
   bounced message and a sloppy impression, and the workspace rule forbids it.

Neither function writes anything or touches the network.
"""

from __future__ import annotations

import re

# Canonical skill -> the spellings that should match it. Order matters only for output.
# The list is deliberately the vocabulary of the postings this search actually sees
# (junior/new-grad software roles in North America), not a general technology taxonomy:
# a term that never appears in these postings is dead weight in the gap column.
SKILL_VOCAB = {
    "TypeScript": (r"type ?script", r"\bts\b"),
    "JavaScript": (r"java ?script", r"(?<![\w.])js(?![\w.])", r"ecmascript"),
    "React": (r"\breact(?:\.?js)?\b", r"\breact native\b"),
    "Next.js": (r"next\.?js"),
    "Vue": (r"\bvue(?:\.js)?\b"),
    "Angular": (r"\bangular\b"),
    "Svelte": (r"\bsvelte\b"),
    "Tailwind": (r"tailwind"),
    "Node.js": (r"node\.?js", r"\bnode\b"),
    "Python": (r"\bpython\b"),
    "Django": (r"\bdjango\b"),
    "Flask": (r"\bflask\b"),
    "FastAPI": (r"fast ?api"),
    "Java": (r"\bjava\b(?! ?script)"),
    "Spring": (r"spring ?boot", r"\bspring\b"),
    "Kotlin": (r"\bkotlin\b"),
    "Scala": (r"\bscala\b"),
    "C++": (r"c\+\+", r"\bcpp\b"),
    "C#": (r"c#", r"\.net\b", r"\bcsharp\b"),
    "C": (r"(?<![a-z+#])c(?![a-z+#])\b(?=[ ,/])"),
    "Go": (r"\bgolang\b", r"(?<![a-z])go(?![a-z])"),
    "Rust": (r"\brust\b"),
    "Ruby": (r"\bruby\b", r"rails"),
    "PHP": (r"\bphp\b"),
    "Swift": (r"\bswift\b"),
    "Kotlin/Android": (r"\bandroid\b"),
    "iOS": (r"\bios\b", r"\bswiftui\b", r"uikit"),
    "SQL": (r"\bsql\b", r"pl/?sql"),
    "PostgreSQL": (r"postgres(?:ql)?", r"\bpostgis\b"),
    "MySQL": (r"\bmysql\b", r"\bmariadb\b"),
    "SQLite": (r"\bsqlite\b"),
    "MongoDB": (r"mongo"),
    "DynamoDB": (r"dynamo ?db"),
    "Redis": (r"\bredis\b"),
    "Elasticsearch": (r"elastic ?search", r"\bopensearch\b"),
    "GraphQL": (r"graph ?ql"),
    "REST APIs": (r"rest(?:ful)? ?apis?", r"\brest\b"),
    "gRPC": (r"\bgrpc\b"),
    "Kafka": (r"\bkafka\b"),
    "RabbitMQ": (r"rabbit ?mq"),
    "Spark": (r"\bspark\b"),
    "Airflow": (r"air ?flow"),
    "dbt": (r"\bdbt\b"),
    "Snowflake": (r"\bsnowflake\b"),
    "Databricks": (r"databricks"),
    "ETL/pipelines": (r"\betl\b", r"data pipelines?", r"\bingest(?:ion|ing)\b"),
    "AWS": (r"\baws\b", r"amazon web services", r"\blambda\b", r"\bs3\b", r"\bec2\b"),
    "GCP": (r"\bgcp\b", r"google cloud", r"\bbigquery\b"),
    "Azure": (r"\bazure\b"),
    "Docker": (r"\bdocker\b", r"containers?"),
    "Kubernetes": (r"kubernetes", r"\bk8s\b"),
    "Terraform": (r"terraform", r"infrastructure as code", r"\biac\b"),
    "CI/CD": (r"ci/?cd", r"continuous (?:integration|delivery|deployment)"),
    "GitHub Actions": (r"github actions"),
    "Jenkins": (r"\bjenkins\b"),
    "GitLab": (r"\bgitlab\b"),
    "Git": (r"(?<![a-z])git(?![a-z])", r"\bgithub\b"),
    "Linux": (r"\blinux\b", r"\bunix\b", r"\bshell\b", r"\bbash\b"),
    "Unit testing": (r"unit tests?", r"unit testing"),
    "Integration testing": (r"integration tests?", r"test automation", r"automated tests?"),
    "Cypress/Selenium": (r"\bcypress\b", r"\bselenium\b", r"\bplaywright\b", r"\bjest\b", r"\bpytest\b"),
    "Debugging": (r"debugging", r"troubleshoot"),
    "Observability": (r"observability", r"monitoring", r"datadog", r"\bsplunk\b", r"prometheus",
                      r"grafana", r"cloudwatch"),
    "Microservices": (r"micro ?services?"),
    "Distributed systems": (r"distributed systems?"),
    "System design": (r"system design", r"software design", r"solution design"),
    "Agile/Scrum": (r"\bagile\b", r"\bscrum\b", r"\bkanban\b"),
    "Code review": (r"code reviews?"),
    "Accessibility": (r"\bwcag\b", r"accessib(?:ility|le)"),
    "Security": (r"secure (?:coding|development|software)", r"application security",
                 r"security best practices", r"\bowasp\b", r"\bwaf\b", r"vulnerabilit"),
    "Auth/identity": (r"\boauth\b", r"\bsso\b", r"\bjwt\b", r"\brbac\b", r"authentication"),
    "Machine learning": (r"machine learning", r"\bml\b", r"pytorch", r"tensorflow", r"scikit"),
    "LLM/agents": (r"\bllms?\b", r"\brag\b", r"prompt engineering", r"ai agents?", r"agentic",
                   r"vector (?:db|database|search|index)", r"embeddings"),
    "Data analysis": (r"data analysis", r"analytics", r"pandas", r"\bsql quer(?:y|ies)\b"),
    "Product ownership": (r"end.to.end ownership", r"own(?:ership)? (?:of )?(?:the )?feature",
                          r"product.?(?:minded|driven|thinking)"),
    "Customer-facing": (r"customer[- ]facing", r"work(?:ing)? (?:directly )?with customers",
                        r"stakeholder"),
    "Frontend": (r"front[- ]?end"),
    "Backend": (r"back[- ]?end"),
    "Full-stack": (r"full[- ]?stack"),
    "Mobile": (r"mobile (?:app|development|platform)", r"\breact native\b", r"\bflutter\b"),
    "Web performance": (r"web performance", r"performance optimi[sz]ation", r"lighthouse"),
    "API design": (r"api design", r"design(?:ing)? apis?"),
    "Data modelling": (r"data model", r"schema design", r"entity resolution"),
}


def _compile():
    out = {}
    for skill, alts in SKILL_VOCAB.items():
        # A single-alternative entry is easy to write as a bare string by accident -- `(r"x")`
        # is a string, not a one-element tuple -- and joining that string's characters with
        # "|" produces a pattern that fails to compile for every skill in the table. Normalise
        # rather than trust the literal.
        if isinstance(alts, str):
            alts = (alts,)
        out[skill] = re.compile("|".join(alts), re.I)
    return out


COMPILED = _compile()

# Words that look like tokens but carry no signal for a resume.
_STOP = {"the", "and", "for", "with", "you", "our", "team", "role", "job"}


def _profile_skills():
    """The skills the candidate's profile already claims, lowercased for matching.

    Read from `candidate/candidate_profile.yaml`, which is the single source of truth for
    every application field. A missing or unreadable profile yields an empty set, which
    makes every keyword a `gap` -- noisy, but never wrong about what the resume says.
    """
    try:
        from .core import load_yaml
        prof = load_yaml("candidate", "candidate_profile.yaml") or {}
    except Exception:  # noqa: BLE001
        return set()
    claimed = set()
    skills = prof.get("skills") or {}
    if isinstance(skills, dict):
        for value in skills.values():
            if isinstance(value, list):
                for item in value:
                    claimed.add(str(item).lower())
    # The target titles are part of the resume's own vocabulary too.
    for title in (prof.get("target") or {}).get("titles") or []:
        claimed.add(str(title).lower())
    return claimed


def _claimed(skill, claimed_skills):
    """Is this canonical skill already spoken for by the profile?

    Matched on the canonical name and on any vocabulary alias, because a profile that says
    "REST APIs" and a vocabulary that says "REST APIs" should agree, while a profile that
    lists only "React" must not silently claim "React Native".
    """
    low = skill.lower()
    if low in claimed_skills:
        return True
    for claimed in claimed_skills:
        if claimed == low or claimed in low or low in claimed:
            return True
    return False


def extract_keywords(text, claimed_skills=None, limit=24):
    """(keywords, have, gap) for a block of posting text.

    `keywords` is every vocabulary hit, most specific first. `have` is the subset the
    profile already claims, `gap` the subset it does not -- the tail of that is what the
    resume should be checked against before applying.
    """
    text = text or ""
    if not text.strip():
        return [], [], []
    claimed = claimed_skills if claimed_skills is not None else _profile_skills()
    hits = [skill for skill, pattern in COMPILED.items() if pattern.search(text)]
    # A posting that says "full-stack" and "frontend" and "backend" is really saying one
    # thing; keep the specific stack terms and drop the generic family words when the
    # specific ones are present.
    if "Full-stack" in hits:
        hits = [h for h in hits if h not in ("Frontend", "Backend")]
    have = [h for h in hits if _claimed(h, claimed)]
    gap = [h for h in hits if h not in have]
    return hits[:limit], have[:limit], gap[:limit]


# Addresses that are never an outreach target, however often they appear in posting text.
_EMAIL_JUNK = re.compile(
    r"(?:no-?reply|donotreply|do-not-reply|example\.(?:com|org)|sentry|wixpress|"
    r"\.png|\.jpe?g|\.gif|\.svg|\.webp|\.css|\.js$|sentry\.io|@2x)", re.I)

_EMAIL = re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")

# Words that precede an address in prose and mean it is not a person's. Not used to drop
# anything -- only to sort the employer's own recruiting inbox behind a named human.
_INBOX_WORDS = ("careers", "jobs", "recruiting", "recruitment", "talent", "hr", "people",
                "apply", "contact", "hello", "info", "admin", "hiring")


def extract_emails(text):
    """Published addresses found in the posting text, best first.

    Best first means a named person's address before the employer's generic recruiting
    inbox: `jane.doe@acme.com` will actually be read, `jobs@acme.com` often will not. The
    address must be literally in the text -- nothing here builds one from a pattern.
    """
    text = text or ""
    found, seen = [], set()
    for hit in _EMAIL.finditer(text):
        addr = hit.group(0).strip().strip(".,;:()<>\"'")
        low = addr.lower()
        if low in seen or _EMAIL_JUNK.search(low):
            continue
        # A path or template follows it, so it is an asset or a placeholder, not a mailbox:
        # "logo@2x.png" style URLs and "/api/v1" route docs both look like addresses.
        tail = text[hit.end():hit.end() + 1]
        if tail in ("/", "{", "}"):
            continue
        seen.add(low)
        found.append(addr)
    def rank(addr):
        local = addr.split("@", 1)[0].lower()
        generic = any(w in local for w in _INBOX_WORDS)
        return (1 if generic else 0, len(local), addr.lower())
    return sorted(found, key=rank)[:4]
