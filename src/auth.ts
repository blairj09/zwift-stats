import http from 'http';
import { exec } from 'child_process';
import readline from 'readline';
import { saveConfig, saveTokens } from './strava.js';

const REDIRECT_URI = 'http://localhost:8888/callback';
const PORT = 8888;

function prompt(question: string): Promise<string> {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  return new Promise(resolve => {
    rl.question(question, answer => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

export async function runAuth(): Promise<void> {
  console.log('\nZwift Stats — Strava OAuth Setup\n');
  console.log('You need a Strava API application. Create one at:');
  console.log('  https://www.strava.com/settings/api\n');
  console.log('Set the "Authorization Callback Domain" to: localhost\n');

  const clientId = await prompt('Enter your Strava Client ID: ');
  const clientSecret = await prompt('Enter your Strava Client Secret: ');

  saveConfig({ clientId, clientSecret });

  const authUrl =
    `https://www.strava.com/oauth/authorize` +
    `?client_id=${clientId}` +
    `&redirect_uri=${encodeURIComponent(REDIRECT_URI)}` +
    `&response_type=code` +
    `&scope=activity:read_all,activity:write`;

  console.log('\nOpening browser for Strava authorization...');
  exec(`open "${authUrl}"`);

  const code = await waitForCallback();
  const tokens = await exchangeCode(clientId, clientSecret, code);
  saveTokens(tokens);

  console.log('\nAuthentication successful! Tokens saved.');
  console.log(`Config stored at: ~/.config/zwift-stats/`);
}

function waitForCallback(): Promise<string> {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      const url = new URL(req.url!, `http://localhost:${PORT}`);
      const code = url.searchParams.get('code');
      const error = url.searchParams.get('error');

      res.writeHead(200, { 'Content-Type': 'text/html' });

      if (error) {
        res.end('<h1>Authorization denied.</h1><p>You can close this tab.</p>');
        server.close();
        reject(new Error(`Strava authorization denied: ${error}`));
        return;
      }

      if (!code) {
        res.end('<h1>No code received.</h1>');
        server.close();
        reject(new Error('No authorization code in callback'));
        return;
      }

      res.end('<h1>Authorized!</h1><p>You can close this tab and return to the terminal.</p>');
      server.close();
      resolve(code);
    });

    server.listen(PORT, () => {
      console.log(`Waiting for Strava callback on port ${PORT}...`);
    });

    server.on('error', reject);
  });
}

async function exchangeCode(
  clientId: string,
  clientSecret: string,
  code: string,
): Promise<{ accessToken: string; refreshToken: string; expiresAt: number }> {
  const res = await fetch('https://www.strava.com/oauth/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      client_id: clientId,
      client_secret: clientSecret,
      code,
      grant_type: 'authorization_code',
    }),
  });

  if (!res.ok) throw new Error(`Token exchange failed: ${res.status} ${await res.text()}`);

  const data = (await res.json()) as any;
  return {
    accessToken: data.access_token,
    refreshToken: data.refresh_token,
    expiresAt: data.expires_at,
  };
}
