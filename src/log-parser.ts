import fs from 'fs';
import os from 'os';
import path from 'path';

export interface RideStats {
  xp: number;
  drops: number;
  endingTotalXp: number;
  endingDrops: number;
}

interface EarnedBlock {
  xp: number;
  drops: number;
  endingTotalXp: number;
  endingDrops: number;
  lineIndex: number;
}

export function parseLog(fitFileName: string, logPath?: string): RideStats | null {
  const logFile = logPath ?? path.join(os.homedir(), 'Documents/Zwift/Logs/Log.txt');

  if (!fs.existsSync(logFile)) {
    console.error(`Log file not found: ${logFile}`);
    return null;
  }

  const lines = fs.readFileSync(logFile, 'utf8').split('\n');

  // Find session start: prefer explicit fitFileNameShort reference, fall back to timestamp in filename
  let sessionStartIdx = lines.findIndex(l => l.includes(`fitFileNameShort: ${fitFileName}`));

  if (sessionStartIdx === -1) {
    // Parse timestamp from filename e.g. "2026-06-01-09-41-22.fit" → "[9:41:22]"
    const m = fitFileName.match(/\d{4}-\d{2}-\d{2}-(\d{2})-(\d{2})-(\d{2})\.fit$/);
    if (m) {
      const timeTag = `[${parseInt(m[1])}:${m[2]}:${m[3]}]`;
      sessionStartIdx = lines.findIndex(l =>
        l.includes(timeTag) && l.includes('Creating New Activity'),
      );
    }
  }

  const searchLines = sessionStartIdx >= 0 ? lines.slice(sessionStartIdx) : lines;
  const blocks = findEarnedBlocks(searchLines);

  // Use the last block with non-zero XP or Drops (skip aborted/cancelled rides)
  const block = [...blocks].reverse().find(b => b.xp > 0 || b.drops > 0);
  if (!block) return null;

  return {
    xp: block.xp,
    drops: block.drops,
    endingTotalXp: block.endingTotalXp,
    endingDrops: block.endingDrops,
  };
}

function findEarnedBlocks(lines: string[]): EarnedBlock[] {
  const blocks: EarnedBlock[] = [];

  for (let i = 0; i < lines.length; i++) {
    const xpMatch = lines[i].match(/^Earned XP\s*:\s*(\d+)/);
    if (!xpMatch) continue;

    // Found an "Earned XP" line — scan the next ~10 lines for companion fields
    const slice = lines.slice(i, i + 12);
    const get = (pattern: RegExp): number => {
      const hit = slice.find(l => pattern.test(l));
      if (!hit) return 0;
      return parseInt(hit.match(pattern)![1], 10);
    };

    blocks.push({
      xp: parseInt(xpMatch[1], 10),
      drops: get(/^Earned drops\s*:\s*(\d+)/),
      endingTotalXp: get(/^Ending Total XP\s*:\s*(\d+)/),
      endingDrops: get(/^Ending Drops\s*:\s*(\d+)/),
      lineIndex: i,
    });
  }

  return blocks;
}
