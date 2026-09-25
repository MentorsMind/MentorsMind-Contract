/// Report writers: JSON baseline + HTML report with trend charts.
extern crate std;

use crate::harness::BenchResult;
use crate::history::HistoryRecord;
use std::fs;
use std::path::Path;

const RESULTS_DIR: &str = "benchmarks/results";

pub fn write_json(results: &[BenchResult]) {
    fs::create_dir_all(RESULTS_DIR).expect("failed to create results dir");
    let path = format!("{}/report.json", RESULTS_DIR);
    let json = serde_json::to_string_pretty(results).expect("failed to serialize results");
    fs::write(&path, json).expect("failed to write report.json");
    println!("📄  JSON report written to {}", path);
}

pub fn write_baseline(results: &[BenchResult], path: &Path) {
    let json = serde_json::to_string_pretty(results).expect("failed to serialize baselines");
    fs::write(path, json).expect("failed to write baselines.json");
    println!("📐  Baseline written to {}", path.display());
}

pub fn write_html(results: &[BenchResult], history: &[HistoryRecord]) {
    fs::create_dir_all(RESULTS_DIR).expect("failed to create results dir");
    let path = format!("{}/report.html", RESULTS_DIR);
    let html = render_html(results, history);
    fs::write(&path, html).expect("failed to write report.html");
    println!("🌐  HTML report written to {}", path);
}

fn render_html(results: &[BenchResult], history: &[HistoryRecord]) -> String {
    let rows = render_results_table(results);
    let chart_section = render_chart_section(results, history);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>MentorsMind Soroban Benchmarks</title>
  <script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
  <style>
    *    {{ box-sizing: border-box; }}
    body {{ font-family: system-ui, sans-serif; margin: 0; padding: 2rem; color: #1a1a2e; background: #f4f4f8; }}
    h1   {{ margin-bottom: 0.25rem; }}
    h2   {{ margin: 2rem 0 0.75rem; font-size: 1.1rem; color: #333; }}
    p.ts {{ color: #555; font-size: 0.85rem; margin-top: 0; }}
    .card {{ background: #fff; border-radius: 8px; box-shadow: 0 2px 6px rgba(0,0,0,.12); overflow: hidden; margin-bottom: 2rem; }}
    table {{ border-collapse: collapse; width: 100%; }}
    th   {{ background: #1a1a2e; color: #fff; padding: 10px 14px; text-align: left; font-size: 0.85rem; letter-spacing: .04em; }}
    td   {{ padding: 8px 14px; font-size: 0.87rem; border-bottom: 1px solid #e8e8f0; }}
    tr:last-child td {{ border-bottom: none; }}
    tr.contract-header td {{ background: #e8e8f0; font-weight: 700; padding: 6px 14px; font-size: 0.8rem; letter-spacing: .08em; text-transform: uppercase; }}
    td.fn {{ font-family: monospace; font-size: 0.85rem; }}
    td.warn {{ color: #c0392b; font-weight: 600; }}
    td.na  {{ color: #aaa; }}
    tr:hover td {{ background: #f0f0fa; }}
    .legend {{ margin-top: 1rem; font-size: 0.82rem; color: #555; padding: 0 1rem 1rem; }}
    .charts-grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(480px, 1fr)); gap: 1.5rem; margin-bottom: 2rem; }}
    .chart-card {{ background: #fff; border-radius: 8px; box-shadow: 0 2px 6px rgba(0,0,0,.12); padding: 1rem 1.25rem 1.25rem; }}
    .chart-card h3 {{ margin: 0 0 0.75rem; font-size: 0.9rem; color: #1a1a2e; font-weight: 600; }}
    canvas {{ max-height: 220px; }}
    .no-history {{ color: #888; font-style: italic; font-size: 0.9rem; }}
  </style>
</head>
<body>
  <h1>🚀 MentorsMind Soroban Benchmarks</h1>
  <p class="ts">Generated: {timestamp}</p>

  <h2>📊 Historical Trends</h2>
  {chart_section}

  <h2>📋 Current Run Results</h2>
  <div class="card">
    <table>
      <thead>
        <tr>
          <th>Entry Point</th>
          <th>CPU Instructions</th>
          <th>Memory (bytes)</th>
          <th>Storage Reads</th>
          <th>Storage Writes</th>
          <th>WASM Size</th>
        </tr>
      </thead>
      <tbody>
        {rows}
      </tbody>
    </table>
    <div class="legend">
      ⚠️ = WASM binary exceeds 64 KB alert threshold &nbsp;|&nbsp;
      N/A = WASM not compiled (run <code>cargo build --target wasm32-unknown-unknown --release</code>)
    </div>
  </div>
</body>
</html>"#,
        timestamp = "see report.json for run metadata",
        chart_section = chart_section,
        rows = rows,
    )
}

fn render_results_table(results: &[BenchResult]) -> String {
    let mut rows = String::new();
    let mut prev_contract = "";

    for r in results {
        if r.contract.as_str() != prev_contract {
            rows.push_str(&format!(
                r#"<tr class="contract-header"><td colspan="6">{}</td></tr>"#,
                html_escape(&r.contract)
            ));
            prev_contract = r.contract.as_str();
        }

        let wasm_cell = if r.wasm_bytes == 0 {
            "<td class=\"na\">N/A</td>".to_string()
        } else if r.wasm_bytes > 64 * 1024 {
            format!("<td class=\"warn\">{} KB ⚠️</td>", r.wasm_bytes / 1024)
        } else {
            format!("<td>{} KB</td>", r.wasm_bytes / 1024)
        };

        rows.push_str(&format!(
            r#"<tr>
  <td class="fn">{entry_point}</td>
  <td>{cpu}</td>
  <td>{mem}</td>
  <td>{reads}</td>
  <td>{writes}</td>
  {wasm}
</tr>"#,
            entry_point = html_escape(&r.entry_point),
            cpu = fmt_num(r.cpu_instructions),
            mem = fmt_num(r.mem_bytes),
            reads = r.storage_reads,
            writes = r.storage_writes,
            wasm = wasm_cell,
        ));
    }
    rows
}

/// Build a Chart.js-powered trend section from historical records.
/// If fewer than 2 history records exist, shows a "no history yet" message.
fn render_chart_section(results: &[BenchResult], history: &[HistoryRecord]) -> String {
    if history.len() < 2 {
        return r#"<p class="no-history">Not enough historical data yet — trends will appear after at least 2 benchmark runs are committed to history.</p>"#.into();
    }

    // Limit to last 30 runs to keep the chart readable.
    let window: Vec<&HistoryRecord> = history.iter().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect();

    // Build a label array for X-axis.
    let labels: Vec<String> = window
        .iter()
        .map(|rec| {
            let short_sha = &rec.sha[..rec.sha.len().min(7)];
            format!("{} ({})", rec.date, short_sha)
        })
        .collect();
    let labels_json = serde_json::to_string(&labels).unwrap_or_default();

    // Build one chart per unique (contract, entry_point) pair from current results.
    let mut charts = String::new();
    let chart_colors = [
        "#4e79a7", "#f28e2b", "#e15759", "#76b7b2",
        "#59a14f", "#edc948", "#b07aa1", "#ff9da7",
    ];

    let mut color_idx = 0;
    let entries: Vec<(&str, &str)> = results
        .iter()
        .map(|r| (r.contract.as_str(), r.entry_point.as_str()))
        .collect();

    for (contract, entry_point) in &entries {
        // Build CPU dataset across history window.
        let cpu_data: Vec<Option<u64>> = window
            .iter()
            .map(|rec| {
                rec.results
                    .iter()
                    .find(|r| r.contract == *contract && r.entry_point == *entry_point)
                    .map(|r| r.cpu_instructions)
            })
            .collect();

        // Skip if all zeros/None (metric not recorded in older history).
        let has_data = cpu_data.iter().any(|v| v.map(|x| x > 0).unwrap_or(false));
        if !has_data {
            continue;
        }

        let cpu_json = serde_json::to_string(
            &cpu_data
                .iter()
                .map(|v| v.unwrap_or(0))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();

        let chart_id = format!("chart_{}", sanitize_id(&format!("{contract}_{entry_point}")));
        let color = chart_colors[color_idx % chart_colors.len()];
        color_idx += 1;

        charts.push_str(&format!(
            r#"<div class="chart-card">
  <h3>{contract} / {entry_point} — CPU Instructions</h3>
  <canvas id="{chart_id}"></canvas>
  <script>
    (function() {{
      var ctx = document.getElementById('{chart_id}').getContext('2d');
      new Chart(ctx, {{
        type: 'line',
        data: {{
          labels: {labels_json},
          datasets: [{{
            label: 'CPU Instructions',
            data: {cpu_json},
            borderColor: '{color}',
            backgroundColor: '{color}22',
            borderWidth: 2,
            pointRadius: 3,
            fill: true,
            tension: 0.3
          }}]
        }},
        options: {{
          responsive: true,
          plugins: {{
            legend: {{ display: false }},
            tooltip: {{ mode: 'index', intersect: false }}
          }},
          scales: {{
            x: {{
              ticks: {{ maxTicksLimit: 8, maxRotation: 30, font: {{ size: 10 }} }}
            }},
            y: {{
              beginAtZero: false,
              ticks: {{ font: {{ size: 10 }} }}
            }}
          }}
        }}
      }});
    }})();
  </script>
</div>"#,
            contract = html_escape(contract),
            entry_point = html_escape(entry_point),
            chart_id = chart_id,
            labels_json = labels_json,
            cpu_json = cpu_json,
            color = color,
        ));
    }

    if charts.is_empty() {
        return r#"<p class="no-history">Historical data found but all CPU metrics are zero — re-run benchmarks with valid baselines to populate trends.</p>"#.into();
    }

    format!(r#"<div class="charts-grid">{}</div>"#, charts)
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
