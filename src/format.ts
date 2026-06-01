import type { CheckedBests } from './bests.js';

const MARKER = '--- Zwift Stats ---';

export function hasMarker(description: string): boolean {
  return description.includes(MARKER);
}

function num(n: number): string {
  return Math.round(n).toLocaleString('en-GB');
}

export function buildBlock(
  xp: number,
  drops: number,
  durationSeconds: number,
  prs: CheckedBests,
): string {
  const hours = durationSeconds / 3600;
  const xpPerHour = xp / hours;
  const dropsPerHour = drops / hours;

  const xpTotalStr = `${prs.newTotalXp ? '🏆 ' : ''}${num(xp)}`;
  const xpHrStr = `${prs.newXpPerHour ? '🏆 ' : ''}${num(xpPerHour)}/hr`;
  const dropsTotalStr = `${prs.newTotalDrops ? '🏆 ' : ''}${num(drops)}`;
  const dropsHrStr = `${prs.newDropsPerHour ? '🏆 ' : ''}${num(dropsPerHour)}/hr`;

  const lines = [
    MARKER,
    `XP: ${xpTotalStr}  (${xpHrStr})`,
    `Drops: ${dropsTotalStr}  (${dropsHrStr})`,
  ];

  return lines.join('\n');
}

export function appendBlock(existingDescription: string, block: string): string {
  const base = existingDescription.trim();
  return base ? `${base}\n\n${block}` : block;
}
