import fs from 'fs';
import os from 'os';
import path from 'path';
import { execSync } from 'child_process';
import { getRides } from './bests.js';

export function generateChart(outputPath?: string): void {
  const rides = getRides();

  if (rides.length === 0) {
    console.log('No rides recorded yet. Process a FIT file first.');
    return;
  }

  const labels = rides.map(r => r.processedAt.slice(0, 10));
  const xpPerRide = rides.map(r => r.xp);
  const dropsPerRide = rides.map(r => r.drops);
  const xpPerHour = rides.map(r => Math.round(r.xpPerHour));

  let cumXp = 0;
  let cumDrops = 0;
  const cumulativeXp = rides.map(r => (cumXp += r.xp));
  const cumulativeDrops = rides.map(r => (cumDrops += r.drops));

  // For bar charts, cap at most recent 50 rides to keep the chart readable
  const barSlice = rides.length > 50 ? rides.length - 50 : 0;
  const barLabels = labels.slice(barSlice);
  const barXp = xpPerRide.slice(barSlice);
  const barXpPerHour = xpPerHour.slice(barSlice);

  const data = {
    labels,
    cumulativeXp,
    cumulativeDrops,
    barLabels,
    barXp,
    barXpPerHour,
  };

  const html = buildHtml(data);
  const dest = outputPath
    ? path.resolve(outputPath.replace('~', os.homedir()))
    : path.join(os.tmpdir(), 'zwift-stats-chart.html');

  fs.writeFileSync(dest, html, 'utf8');
  console.log(`Chart written to: ${dest}`);
  try {
    execSync(`open "${dest}"`);
  } catch {
    console.log('Could not open browser automatically. Open the file above manually.');
  }
}

interface ChartData {
  labels: string[];
  cumulativeXp: number[];
  cumulativeDrops: number[];
  barLabels: string[];
  barXp: number[];
  barXpPerHour: number[];
}

function buildHtml(data: ChartData): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Zwift Stats</title>
  <script src="https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js"></script>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: #0d0d1a;
      color: #e0e0e0;
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
      padding: 24px;
    }
    h1 {
      text-align: center;
      font-size: 1.6rem;
      font-weight: 600;
      color: #ff6b00;
      margin-bottom: 24px;
      letter-spacing: 0.05em;
    }
    .grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 24px;
    }
    .card {
      background: #1a1a2e;
      border-radius: 12px;
      padding: 20px;
      border: 1px solid #2a2a4a;
    }
    .card h2 {
      font-size: 0.85rem;
      font-weight: 500;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: #888;
      margin-bottom: 14px;
    }
    .chart-wrap { position: relative; height: 260px; }
    @media (max-width: 700px) { .grid { grid-template-columns: 1fr; } }
  </style>
</head>
<body>
  <h1>Zwift Stats</h1>
  <div class="grid">
    <div class="card">
      <h2>Cumulative XP over time</h2>
      <div class="chart-wrap"><canvas id="cXp"></canvas></div>
    </div>
    <div class="card">
      <h2>Cumulative Drops over time</h2>
      <div class="chart-wrap"><canvas id="cDrops"></canvas></div>
    </div>
    <div class="card">
      <h2>XP per ride (last 50)</h2>
      <div class="chart-wrap"><canvas id="xpBar"></canvas></div>
    </div>
    <div class="card">
      <h2>XP/hr efficiency (last 50)</h2>
      <div class="chart-wrap"><canvas id="effLine"></canvas></div>
    </div>
  </div>

  <script>
    const DATA = ${JSON.stringify(data)};

    const GRID_COLOR = 'rgba(255,255,255,0.06)';
    const TICK_COLOR = '#666';

    const baseScales = {
      x: {
        ticks: { color: TICK_COLOR, maxTicksLimit: 8, maxRotation: 0 },
        grid: { color: GRID_COLOR },
      },
      y: {
        ticks: {
          color: TICK_COLOR,
          callback: v => Number(v).toLocaleString('en-GB'),
        },
        grid: { color: GRID_COLOR },
      },
    };

    const basePlugins = {
      legend: { display: false },
      tooltip: {
        callbacks: {
          label: ctx => ' ' + Number(ctx.parsed.y).toLocaleString('en-GB'),
        },
      },
    };

    function lineDataset(data, color) {
      return {
        data,
        borderColor: color,
        backgroundColor: color + '22',
        fill: true,
        borderWidth: 2,
        pointRadius: data.length > 60 ? 0 : 3,
        tension: 0.3,
      };
    }

    new Chart(document.getElementById('cXp'), {
      type: 'line',
      data: { labels: DATA.labels, datasets: [lineDataset(DATA.cumulativeXp, '#ff6b00')] },
      options: { responsive: true, maintainAspectRatio: false, scales: baseScales, plugins: basePlugins },
    });

    new Chart(document.getElementById('cDrops'), {
      type: 'line',
      data: { labels: DATA.labels, datasets: [lineDataset(DATA.cumulativeDrops, '#00d4aa')] },
      options: { responsive: true, maintainAspectRatio: false, scales: baseScales, plugins: basePlugins },
    });

    new Chart(document.getElementById('xpBar'), {
      type: 'bar',
      data: {
        labels: DATA.barLabels,
        datasets: [{
          data: DATA.barXp,
          backgroundColor: '#ff6b00bb',
          borderColor: '#ff6b00',
          borderWidth: 1,
          borderRadius: 3,
        }],
      },
      options: { responsive: true, maintainAspectRatio: false, scales: baseScales, plugins: basePlugins },
    });

    new Chart(document.getElementById('effLine'), {
      type: 'line',
      data: { labels: DATA.barLabels, datasets: [lineDataset(DATA.barXpPerHour, '#c87aff')] },
      options: { responsive: true, maintainAspectRatio: false, scales: baseScales, plugins: basePlugins },
    });
  </script>
</body>
</html>`;
}
