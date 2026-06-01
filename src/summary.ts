import { getRides, getAllBests } from './bests.js';

function n(value: number): string {
  return value.toLocaleString('en-GB');
}

function dur(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return `${h}h ${m}m`;
}

function shortDate(iso: string): string {
  return iso.slice(0, 10);
}

export function printStats(): void {
  const rides = getRides();

  if (rides.length === 0) {
    console.log('No rides recorded yet. Process a FIT file first.');
    return;
  }

  const bests = getAllBests();

  const totalXp = rides.reduce((s, r) => s + r.xp, 0);
  const totalDrops = rides.reduce((s, r) => s + r.drops, 0);
  const totalSeconds = rides.reduce((s, r) => s + r.durationSeconds, 0);
  const avgXp = Math.round(totalXp / rides.length);
  const avgDrops = Math.round(totalDrops / rides.length);
  const avgXpPerHour = Math.round(rides.reduce((s, r) => s + r.xpPerHour, 0) / rides.length);
  const avgDropsPerHour = Math.round(rides.reduce((s, r) => s + r.dropsPerHour, 0) / rides.length);
  const firstDate = shortDate(rides[0].processedAt);
  const lastDate = shortDate(rides[rides.length - 1].processedAt);

  console.log('\n=== Zwift Stats Summary ===\n');

  console.log(`Lifetime  (${rides.length} rides · ${firstDate} – ${lastDate})`);
  console.log(`  Total XP:     ${n(totalXp).padStart(12)}   avg ${n(avgXp)}/ride · ${n(avgXpPerHour)}/hr avg`);
  console.log(`  Total Drops:  ${n(totalDrops).padStart(12)}   avg ${n(avgDrops)}/ride · ${n(avgDropsPerHour)}/hr avg`);
  console.log(`  Total time:   ${dur(totalSeconds)}`);

  if (bests.length > 0) {
    console.log('\nPersonal Bests');
    const bestMap = Object.fromEntries(bests.map(b => [b.metric, b]));
    const labels: Record<string, string> = {
      xp_per_hour: 'XP/hr',
      drops_per_hour: 'Drops/hr',
      total_xp: 'Total XP',
      total_drops: 'Total Drops',
    };
    for (const [metric, label] of Object.entries(labels)) {
      const b = bestMap[metric];
      if (!b) continue;
      const name = b.rideName ? `"${b.rideName}"` : 'unknown ride';
      const date = shortDate(b.achievedAt);
      console.log(`  ${label.padEnd(14)} ${n(Math.round(b.value)).padStart(12)}  (${name} — ${date})`);
    }
  }

  const thirtyDaysAgo = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
  const recent = rides.filter(r => r.processedAt >= thirtyDaysAgo);
  if (recent.length > 0) {
    const recentXp = recent.reduce((s, r) => s + r.xp, 0);
    const recentDrops = recent.reduce((s, r) => s + r.drops, 0);
    console.log(`\nLast 30 days  (${recent.length} rides)`);
    console.log(`  XP:     ${n(recentXp).padStart(12)}   avg ${n(Math.round(recentXp / recent.length))}/ride`);
    console.log(`  Drops:  ${n(recentDrops).padStart(12)}   avg ${n(Math.round(recentDrops / recent.length))}/ride`);
  }

  // Monthly breakdown — last 6 months
  const monthlyMap: Record<string, { rides: number; xp: number; drops: number }> = {};
  for (const r of rides) {
    const month = r.processedAt.slice(0, 7);
    if (!monthlyMap[month]) monthlyMap[month] = { rides: 0, xp: 0, drops: 0 };
    monthlyMap[month].rides++;
    monthlyMap[month].xp += r.xp;
    monthlyMap[month].drops += r.drops;
  }
  const months = Object.keys(monthlyMap).sort().slice(-6);
  if (months.length > 0) {
    console.log('\nMonthly breakdown (last 6 months)');
    console.log('  Month        Rides    XP              Drops');
    for (const month of months) {
      const m = monthlyMap[month];
      console.log(
        `  ${month}    ${String(m.rides).padStart(4)}   ${n(m.xp).padStart(10)}   ${n(m.drops).padStart(12)}`,
      );
    }
  }

  console.log('');
}
