// Shapes of what /api/dashboard and /api/posting hand over. The server is Python and owns the
// truth; these are the fields the client actually reads.
export interface Posting {
  id: string;
  company: string;
  title: string;
  location: string;
  tier: string;
  url: string;
  posted?: string;
  posted_derived?: string;
  scanned_at?: string;
  age_days?: number | null;
  expires_in_days?: number | null;
  keywords?: string[];
  score?: number | null;
  salary_annual_min?: number | null;
  salary_annual_max?: number | null;
  level_ii?: boolean;
  requires_us_work_auth?: boolean;
  below_comp_floor?: boolean;
  emails?: string[];
  verified_emails?: string[];
  skills_have?: string[];
  skills_gap?: string[];
  source?: string;
  ats?: string;
}

export interface AppliedRow {
  id: string;
  title: string;
  company: string;
  location: string;
  url: string;
  tier?: string;
  applied_at: string;
}

export interface Reword {
  where?: string;
  before?: string;
  after?: string;
  advice?: string;
  note?: string;
}

export interface Contact {
  name?: string;
  role?: string;
  bio?: string;
  email?: string;
  source?: string;
  confidence?: string;
}

export interface Brief {
  status?: string;
  error?: string;
  verdict?: string;
  key_skills?: string[];
  tailoring?: {
    lead_with?: string;
    reword?: (Reword | string)[];
    missing?: string[];
    notes?: string;
  };
  contacts?: Contact[];
  contacts_source?: string;
  contact_notes?: string;
  reviewed?: boolean | string;
  review_error?: string;
  generated_at?: string;
  elapsed_s?: number | string;
  posting_id?: string;
}

export interface Dashboard {
  ran_at?: string;
  tiers: string[];
  postings: Posting[];
  applied: AppliedRow[];
  // health page only
  boards_total?: number;
  boards_ok?: number;
  boards_failing?: number;
  matched?: number;
  collected?: number;
  window?: number;
  sweep?: {count?: number; verdict?: string};
  source_seconds?: Record<string, number>;
  sources?: Record<string, number>;
  source_total?: number | string;
  failing?: {
    name: string;
    consecutive_failures: number;
    error_kind: string;
    last_error: string;
    next_check_at?: string;
  }[];
}

export interface Detail {
  error?: string;
  posting?: Posting;
  description?: string;
  description_length?: number;
  running?: boolean;
  stale?: boolean;
  has_script?: boolean;
  brief?: Brief;
}

export interface AppliedResponse {
  applied?: AppliedRow[];
}
