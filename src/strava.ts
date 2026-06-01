import fs from 'fs';
import os from 'os';
import path from 'path';

const CONFIG_DIR = path.join(os.homedir(), '.config', 'zwift-stats');
const TOKENS_PATH = path.join(CONFIG_DIR, 'tokens.json');
const CONFIG_PATH = path.join(CONFIG_DIR, 'config.json');

export interface Config {
  clientId: string;
  clientSecret: string;
}

export interface Tokens {
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
}

export function readConfig(): Config {
  if (!fs.existsSync(CONFIG_PATH)) {
    throw new Error(`Config not found. Run 'zwift-stats auth' first.\nExpected: ${CONFIG_PATH}`);
  }
  return JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf8'));
}

export function saveConfig(config: Config): void {
  fs.mkdirSync(CONFIG_DIR, { recursive: true });
  fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2));
}

export function readTokens(): Tokens {
  if (!fs.existsSync(TOKENS_PATH)) {
    throw new Error(`Not authenticated. Run 'zwift-stats auth' first.`);
  }
  return JSON.parse(fs.readFileSync(TOKENS_PATH, 'utf8'));
}

export function saveTokens(tokens: Tokens): void {
  fs.mkdirSync(CONFIG_DIR, { recursive: true });
  fs.writeFileSync(TOKENS_PATH, JSON.stringify(tokens, null, 2));
}

async function refreshAccessToken(config: Config, tokens: Tokens): Promise<Tokens> {
  const res = await fetch('https://www.strava.com/oauth/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      client_id: config.clientId,
      client_secret: config.clientSecret,
      grant_type: 'refresh_token',
      refresh_token: tokens.refreshToken,
    }),
  });

  if (!res.ok) throw new Error(`Token refresh failed: ${res.status} ${await res.text()}`);

  const data = (await res.json()) as any;
  const updated: Tokens = {
    accessToken: data.access_token,
    refreshToken: data.refresh_token,
    expiresAt: data.expires_at,
  };
  saveTokens(updated);
  return updated;
}

async function getValidToken(): Promise<string> {
  const config = readConfig();
  let tokens = readTokens();

  // Refresh if expiring within 5 minutes
  if (Date.now() / 1000 > tokens.expiresAt - 300) {
    tokens = await refreshAccessToken(config, tokens);
  }

  return tokens.accessToken;
}

export interface StravaActivity {
  id: number;
  name: string;
  sport_type: string;
  start_date: string;
  description: string | null;
  external_id?: string;
}

export async function findActivity(startTime: Date): Promise<StravaActivity | null> {
  const token = await getValidToken();
  const epochStart = Math.floor(startTime.getTime() / 1000);
  const after = epochStart - 300;
  const before = epochStart + 300;

  const maxAttempts = 5;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    const res = await fetch(
      `https://www.strava.com/api/v3/athlete/activities?after=${after}&before=${before}&per_page=10`,
      { headers: { Authorization: `Bearer ${token}` } },
    );

    if (!res.ok) throw new Error(`Strava activities fetch failed: ${res.status}`);

    const activities = (await res.json()) as StravaActivity[];
    const candidates = activities.filter(a => a.sport_type === 'VirtualRide');

    if (candidates.length === 0) {
      if (attempt < maxAttempts) {
        console.log(`Activity not found yet (attempt ${attempt}/${maxAttempts}), retrying in 30s...`);
        await sleep(30_000);
      }
      continue;
    }

    if (candidates.length === 1) return candidates[0];

    // Multiple VirtualRides in the window — fetch full details to find the Zwift upload
    console.log(`  Found ${candidates.length} VirtualRide activities — checking external_id to identify Zwift upload...`);
    const detailed = await Promise.all(candidates.map(c => getActivity(c.id, token)));
    const zwiftActivity = detailed.find(a => a.external_id?.startsWith('zwift-activity-'));

    if (zwiftActivity) return zwiftActivity;

    // Fallback: no Zwift external_id found — warn and use the first candidate
    console.warn('  Warning: could not identify Zwift upload by external_id, using first candidate.');
    return candidates[0];
  }

  return null;
}

export async function getActivity(activityId: number, token?: string): Promise<StravaActivity> {
  const t = token ?? (await getValidToken());
  const res = await fetch(`https://www.strava.com/api/v3/activities/${activityId}`, {
    headers: { Authorization: `Bearer ${t}` },
  });
  if (!res.ok) throw new Error(`Strava get activity failed: ${res.status}`);
  return (await res.json()) as StravaActivity;
}

export async function updateDescription(activityId: number, description: string): Promise<void> {
  const token = await getValidToken();
  const res = await fetch(`https://www.strava.com/api/v3/activities/${activityId}`, {
    method: 'PUT',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ description }),
  });
  if (!res.ok) throw new Error(`Strava update failed: ${res.status} ${await res.text()}`);
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
