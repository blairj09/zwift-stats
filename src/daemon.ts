import fs from 'fs';
import os from 'os';
import path from 'path';
import { execSync } from 'child_process';

const PLIST_LABEL = 'com.zwift-stats';
const PLIST_DEST = path.join(os.homedir(), 'Library', 'LaunchAgents', `${PLIST_LABEL}.plist`);

function findNode(): string {
  try {
    return execSync('which node', { encoding: 'utf8' }).trim();
  } catch {
    // Common Homebrew locations
    for (const p of ['/opt/homebrew/bin/node', '/usr/local/bin/node']) {
      if (fs.existsSync(p)) return p;
    }
    throw new Error('Could not find node binary. Make sure Node.js is installed.');
  }
}

function generatePlist(projectDir: string): string {
  const node = findNode();
  const home = os.homedir();
  const tsx = path.join(projectDir, 'node_modules', '.bin', 'tsx');
  const entry = path.join(projectDir, 'src', 'index.ts');
  const log = path.join(home, 'Library', 'Logs', 'zwift-stats.log');

  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${PLIST_LABEL}</string>

    <key>ProgramArguments</key>
    <array>
        <string>${node}</string>
        <string>${tsx}</string>
        <string>${entry}</string>
        <string>start</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>${home}</string>
        <key>PATH</key>
        <string>${path.dirname(node)}:/usr/local/bin:/usr/bin:/bin</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>${log}</string>

    <key>StandardErrorPath</key>
    <string>${log}</string>
</dict>
</plist>
`;
}

export function installDaemon(): void {
  const projectDir = path.resolve(__dirname, '..');

  if (!fs.existsSync(path.join(projectDir, 'node_modules', '.bin', 'tsx'))) {
    throw new Error(`node_modules not found. Run 'npm install' in ${projectDir} first.`);
  }

  const plist = generatePlist(projectDir);
  fs.mkdirSync(path.dirname(PLIST_DEST), { recursive: true });
  fs.writeFileSync(PLIST_DEST, plist);
  console.log(`Plist written to: ${PLIST_DEST}`);

  try {
    execSync(`launchctl unload "${PLIST_DEST}" 2>/dev/null; true`);
    execSync(`launchctl load "${PLIST_DEST}"`);
    console.log('Daemon loaded and running.');
    console.log(`Logs: ~/Library/Logs/zwift-stats.log`);
  } catch (err: any) {
    throw new Error(`launchctl failed: ${err.message}`);
  }
}

export function uninstallDaemon(): void {
  if (!fs.existsSync(PLIST_DEST)) {
    console.log('Daemon is not installed.');
    return;
  }
  execSync(`launchctl unload "${PLIST_DEST}" 2>/dev/null; true`);
  fs.unlinkSync(PLIST_DEST);
  console.log('Daemon unloaded and plist removed.');
}

export function daemonStatus(): void {
  try {
    const out = execSync('launchctl list', { encoding: 'utf8' });
    const line = out.split('\n').find(l => l.includes(PLIST_LABEL));
    if (!line) {
      console.log('Daemon is not loaded.');
      return;
    }
    const [pid, status] = line.trim().split(/\s+/);
    if (pid !== '-') {
      console.log(`Daemon is running (PID ${pid}).`);
    } else {
      console.log(`Daemon is loaded but not running (last exit status: ${status}).`);
    }
    console.log(`Logs: ~/Library/Logs/zwift-stats.log`);
  } catch {
    console.log('Could not determine daemon status.');
  }
}
