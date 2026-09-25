use state_transition_analyzer::{StateMachine, generate_mermaid_diagram, generate_markdown_report, ProtocolCoverageReport};
use serde_json::to_string_pretty;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn main() {
    println!("🔍 Analyzing state machine transitions...\n");

    // Define all state machines with their valid transitions
    let escrow_sm = StateMachine::new(
        "EscrowStatus".to_string(),
        vec![
            "Pending".to_string(),
            "Active".to_string(),
            "Released".to_string(),
            "Disputed".to_string(),
            "Refunded".to_string(),
            "Resolved".to_string(),
        ],
        vec![
            ("Pending".to_string(), "Active".to_string()),
            ("Pending".to_string(), "Refunded".to_string()),
            ("Active".to_string(), "Released".to_string()),
            ("Active".to_string(), "Disputed".to_string()),
            ("Active".to_string(), "Refunded".to_string()),
            ("Disputed".to_string(), "Resolved".to_string()),
            ("Disputed".to_string(), "Refunded".to_string()),
        ],
    );

    let subscription_sm = StateMachine::new(
        "SubscriptionStatus".to_string(),
        vec![
            "Trial".to_string(),
            "Active".to_string(),
            "GracePeriod".to_string(),
            "Paused".to_string(),
            "Cancelled".to_string(),
            "Expired".to_string(),
        ],
        vec![
            ("Trial".to_string(), "Active".to_string()),
            ("Trial".to_string(), "Cancelled".to_string()),
            ("Active".to_string(), "GracePeriod".to_string()),
            ("Active".to_string(), "Paused".to_string()),
            ("Active".to_string(), "Cancelled".to_string()),
            ("GracePeriod".to_string(), "Active".to_string()),
            ("GracePeriod".to_string(), "Expired".to_string()),
            ("Paused".to_string(), "Active".to_string()),
            ("Paused".to_string(), "Cancelled".to_string()),
        ],
    );

    let loan_sm = StateMachine::new(
        "LoanStatus".to_string(),
        vec![
            "Pending".to_string(),
            "Active".to_string(),
            "Repaid".to_string(),
            "Defaulted".to_string(),
            "Cancelled".to_string(),
        ],
        vec![
            ("Pending".to_string(), "Active".to_string()),
            ("Pending".to_string(), "Cancelled".to_string()),
            ("Active".to_string(), "Repaid".to_string()),
            ("Active".to_string(), "Defaulted".to_string()),
        ],
    );

    let isa_sm = StateMachine::new(
        "ISAStatus".to_string(),
        vec![
            "Pending".to_string(),
            "StudyPeriod".to_string(),
            "GracePeriod".to_string(),
            "Repayment".to_string(),
            "Completed".to_string(),
            "Defaulted".to_string(),
        ],
        vec![
            ("Pending".to_string(), "StudyPeriod".to_string()),
            ("StudyPeriod".to_string(), "GracePeriod".to_string()),
            ("GracePeriod".to_string(), "Repayment".to_string()),
            ("Repayment".to_string(), "Completed".to_string()),
            ("Repayment".to_string(), "Defaulted".to_string()),
        ],
    );

    let state_machines = vec![escrow_sm, subscription_sm, loan_sm, isa_sm];

    // Simulate tested transitions (based on existing tests)
    let mut tested_transitions = HashSet::new();
    
    // Escrow tested transitions
    tested_transitions.insert(("Pending".to_string(), "Active".to_string()));
    tested_transitions.insert(("Active".to_string(), "Released".to_string()));
    tested_transitions.insert(("Active".to_string(), "Disputed".to_string()));
    tested_transitions.insert(("Disputed".to_string(), "Resolved".to_string()));
    tested_transitions.insert(("Pending".to_string(), "Refunded".to_string()));
    
    // Subscription tested transitions
    tested_transitions.insert(("Trial".to_string(), "Active".to_string()));
    tested_transitions.insert(("Active".to_string(), "GracePeriod".to_string()));
    tested_transitions.insert(("Active".to_string(), "Cancelled".to_string()));
    
    // Loan tested transitions
    tested_transitions.insert(("Pending".to_string(), "Active".to_string()));
    tested_transitions.insert(("Active".to_string(), "Repaid".to_string()));
    
    // ISA tested transitions
    tested_transitions.insert(("Pending".to_string(), "StudyPeriod".to_string()));
    tested_transitions.insert(("GracePeriod".to_string(), "Repayment".to_string()));

    // Generate coverage reports
    let mut coverage_reports = Vec::new();
    for sm in &state_machines {
        let report = sm.analyze_coverage(tested_transitions.clone());
        println!("📊 {}: {:.2}% coverage ({}/{})",
                 sm.name,
                 report.coverage_percentage,
                 report.tested_transitions,
                 report.valid_transitions);
        coverage_reports.push(report);
    }

    // Calculate overall coverage
    let total_valid: usize = coverage_reports.iter().map(|r| r.valid_transitions).sum();
    let total_tested: usize = coverage_reports.iter().map(|r| r.tested_transitions).sum();
    let overall_coverage = if total_valid > 0 {
        (total_tested as f64 / total_valid as f64) * 100.0
    } else {
        100.0
    };

    println!("\n📈 Overall Coverage: {:.2}% ({}/{})\n", overall_coverage, total_tested, total_valid);

    // Create output directory
    let output_dir = "analysis_output";
    fs::create_dir_all(output_dir).expect("Failed to create output directory");

    // Generate JSON report
    let protocol_report = ProtocolCoverageReport {
        generated_at: "2026-06-16T03:14:39Z".to_string(),
        state_machines: coverage_reports.clone(),
        overall_coverage,
    };

    let json_report = to_string_pretty(&protocol_report).expect("Failed to serialize JSON");
    fs::write(
        Path::new(output_dir).join("coverage_report.json"),
        json_report,
    ).expect("Failed to write JSON report");
    println!("✅ JSON report written to {}/coverage_report.json", output_dir);

    // Generate Markdown report
    let markdown = generate_markdown_report(&coverage_reports);
    fs::write(
        Path::new(output_dir).join("coverage_report.md"),
        markdown,
    ).expect("Failed to write Markdown report");
    println!("✅ Markdown report written to {}/coverage_report.md", output_dir);

    // Generate Mermaid diagrams
    for sm in &state_machines {
        let diagram = generate_mermaid_diagram(sm);
        let filename = format!("{}_diagram.mmd", sm.name.to_lowercase());
        fs::write(
            Path::new(output_dir).join(&filename),
            diagram,
        ).expect("Failed to write Mermaid diagram");
        println!("✅ Diagram written to {}/{}", output_dir, filename);
    }

    println!("\n✨ Analysis complete!");
}
