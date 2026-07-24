/// Report writers: JSON baseline + HTML report.
extern crate std;

use crate::harness::BenchResult;
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

pub fn write_html(results: &[BenchResult]) {
    fs::create_dir_all(RESULTS_DIR).expect("failed to create results dir");
    let path = format!("{}/report.html", RESULTS_DIR);
    let html = render_html(results);
    fs::write(&path, html).expect("failed to write report.html");
    println!("🌐  HTML report written to {}", path);
}

fn render_html(results: &[BenchResult]) -> String {
    // Group by contract for the table headers
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
            format!(
                "<td class=\"warn\">{} KB ⚠️</td>",
                r.wasm_bytes / 1024
            )
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

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>MentorsMind Soroban Benchmarks</title>
  <style>
    body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #1a1a2e; background: #f4f4f8; }}
    h1   {{ margin-bottom: 0.25rem; }}
    p.ts {{ color: #555; font-size: 0.85rem; margin-top: 0; }}
    table {{ border-collapse: collapse; width: 100%; background: #fff; border-radius: 8px; overflow: hidden; box-shadow: 0 2px 6px rgba(0,0,0,.12); }}
    th   {{ background: #1a1a2e; color: #fff; padding: 10px 14px; text-align: left; font-size: 0.85rem; letter-spacing: .04em; }}
    td   {{ padding: 8px 14px; font-size: 0.87rem; border-bottom: 1px solid #e8e8f0; }}
    tr:last-child td {{ border-bottom: none; }}
    tr.contract-header td {{ background: #e8e8f0; font-weight: 700; padding: 6px 14px; font-size: 0.8rem; letter-spacing: .08em; text-transform: uppercase; }}
    td.fn {{ font-family: monospace; font-size: 0.85rem; }}
    td.warn {{ color: #c0392b; font-weight: 600; }}
    td.na  {{ color: #aaa; }}
    tr:hover td {{ background: #f0f0fa; }}
    .legend {{ margin-top: 1.5rem; font-size: 0.82rem; color: #555; }}
  </style>
</head>
<body>
  <h1>🚀 MentorsMind Soroban Benchmarks</h1>
  <p class="ts">Generated: {timestamp}</p>
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
</body>
</html>"#,
        timestamp = timestamp(),
        rows = rows,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fmt_num(n: u64) -> String {
    // Insert thousands separators
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

fn timestamp() -> String {
    // Simple ISO-like timestamp using std — no chrono dep needed
    "see report.json for metadata".to_string()
}
