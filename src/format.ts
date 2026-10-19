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
  distanceKm?: number,
): string {
  const hours = durationSeconds / 3600;
  const xpPerHour = xp / hours;
  const dropsPerHour = drops / hours;
  const trophy = (isNew: boolean) => (isNew ? '🏆 ' : '');

  const xpRates = [`${trophy(prs.newXpPerHour)}${num(xpPerHour)}/hr`];
  const dropsRates = [`${trophy(prs.newDropsPerHour)}${num(dropsPerHour)}/hr`];
  if (distanceKm && distanceKm > 0) {
    xpRates.push(`${trophy(prs.newXpPerKm)}${num(xp / distanceKm)}/km`);
    dropsRates.push(`${trophy(prs.newDropsPerKm)}${num(drops / distanceKm)}/km`);
  }

  const lines = [
    MARKER,
    `XP: ${trophy(prs.newTotalXp)}${num(xp)}  (${xpRates.join(' · ')})`,
    `Drops: ${trophy(prs.newTotalDrops)}${num(drops)}  (${dropsRates.join(' · ')})`,
  ];

  return lines.join('\n');
}

export function appendBlock(existingDescription: string, block: string): string {
  const base = existingDescription.trim();
  return base ? `${base}\n\n${block}` : block;
}
