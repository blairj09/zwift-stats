import { getRides, getAllBests, rideDate, type RideRecord } from './bests.js';

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

/** Per-km rate over rides that have a recorded distance. */
function perKm(rides: RideRecord[], value: (r: RideRecord) => number): number | null {
  const withKm = rides.filter(r => r.distanceKm && r.distanceKm > 0);
  const km = withKm.reduce((s, r) => s + r.distanceKm!, 0);
  return km > 0 ? withKm.reduce((s, r) => s + value(r), 0) / km : null;
}

/** Monday of the ride's week in local time, as YYYY-MM-DD. */
function weekOf(iso: string): string {
  const d = new Date(iso);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  const pad = (x: number) => String(x).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
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
  const firstDate = shortDate(rideDate(rides[0]));
  const lastDate = shortDate(rideDate(rides[rides.length - 1]));
  const totalKm = rides.reduce((s, r) => s + (r.distanceKm ?? 0), 0);
  const xpPerKm = perKm(rides, r => r.xp);
  const dropsPerKm = perKm(rides, r => r.drops);
  const kmStr = (v: number | null) => (v === null ? '' : ` · ${n(Math.round(v))}/km`);

  console.log('\n=== Zwift Stats Summary ===\n');

  console.log(`Lifetime  (${rides.length} rides · ${firstDate} – ${lastDate})`);
  console.log(`  Total XP:     ${n(totalXp).padStart(12)}   avg ${n(avgXp)}/ride · ${n(avgXpPerHour)}/hr avg${kmStr(xpPerKm)}`);
  console.log(`  Total Drops:  ${n(totalDrops).padStart(12)}   avg ${n(avgDrops)}/ride · ${n(avgDropsPerHour)}/hr avg${kmStr(dropsPerKm)}`);
  console.log(`  Total time:   ${dur(totalSeconds)}`);
  if (totalKm > 0) console.log(`  Distance:     ${n(Math.round(totalKm))} km`);

  if (bests.length > 0) {
    console.log('\nPersonal Bests');
    const bestMap = Object.fromEntries(bests.map(b => [b.metric, b]));
    const labels: Record<string, string> = {
      xp_per_hour: 'XP/hr',
      drops_per_hour: 'Drops/hr',
      xp_per_km: 'XP/km',
      drops_per_km: 'Drops/km',
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
  const recent = rides.filter(r => rideDate(r) >= thirtyDaysAgo);
  if (recent.length > 0) {
    const recentXp = recent.reduce((s, r) => s + r.xp, 0);
    const recentDrops = recent.reduce((s, r) => s + r.drops, 0);
    console.log(`\nLast 30 days  (${recent.length} rides)`);
    console.log(`  XP:     ${n(recentXp).padStart(12)}   avg ${n(Math.round(recentXp / recent.length))}/ride`);
    console.log(`  Drops:  ${n(recentDrops).padStart(12)}   avg ${n(Math.round(recentDrops / recent.length))}/ride`);
  }

  // Weekly breakdown — last 8 weeks
  const weeklyMap: Record<string, RideRecord[]> = {};
  for (const r of rides) (weeklyMap[weekOf(rideDate(r))] ??= []).push(r);
  const weeks = Object.keys(weeklyMap).sort().slice(-8);
  if (weeks.length > 0) {
    console.log('\nWeekly breakdown (last 8 weeks, starting Monday)');
    console.log('  Week of       Rides       km        XP   XP/hr   XP/km        Drops  Drops/hr  Drops/km');
    for (const week of weeks) {
      const w = weeklyMap[week];
      const xp = w.reduce((s, r) => s + r.xp, 0);
      const drops = w.reduce((s, r) => s + r.drops, 0);
      const hrs = w.reduce((s, r) => s + r.durationSeconds, 0) / 3600;
      const km = w.reduce((s, r) => s + (r.distanceKm ?? 0), 0);
      const wXpKm = perKm(w, r => r.xp);
      const wDropsKm = perKm(w, r => r.drops);
      const cell = (v: number | null, width: number) => (v === null ? '–' : n(Math.round(v))).padStart(width);
      console.log(
        `  ${week}   ${String(w.length).padStart(5)} ${cell(km || null, 8)} ${cell(xp, 9)} ${cell(xp / hrs, 7)} ${cell(wXpKm, 7)} ${cell(drops, 12)} ${cell(drops / hrs, 9)} ${cell(wDropsKm, 9)}`,
      );
    }
  }

  // Monthly breakdown — last 6 months
  const monthlyMap: Record<string, { rides: number; xp: number; drops: number }> = {};
  for (const r of rides) {
    const month = rideDate(r).slice(0, 7);
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
