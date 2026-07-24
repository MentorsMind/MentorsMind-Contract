use serde::{Serialize, Deserialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct StateMachine {
    pub name: String,
    pub states: Vec<String>,
    pub valid_transitions: Vec<(String, String)>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TransitionCoverageReport {
    pub state_machine: String,
    pub total_possible_transitions: usize,
    pub valid_transitions: usize,
    pub tested_transitions: usize,
    pub coverage_percentage: f64,
    pub untested_transitions: Vec<(String, String)>,
    pub all_valid_transitions: Vec<(String, String)>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProtocolCoverageReport {
    pub generated_at: String,
    pub state_machines: Vec<TransitionCoverageReport>,
    pub overall_coverage: f64,
}

impl StateMachine {
    pub fn new(name: String, states: Vec<String>, valid_transitions: Vec<(String, String)>) -> Self {
        StateMachine {
            name,
            states,
            valid_transitions,
        }
    }

    pub fn analyze_coverage(&self, tested_transitions: HashSet<(String, String)>) -> TransitionCoverageReport {
        let total_possible = self.states.len() * self.states.len();
        let valid_count = self.valid_transitions.len();
        let tested_count = self.valid_transitions.iter()
            .filter(|t| tested_transitions.contains(t))
            .count();

        let untested: Vec<_> = self.valid_transitions.iter()
            .filter(|t| !tested_transitions.contains(t))
            .cloned()
            .collect();

        let coverage = if valid_count > 0 {
            (tested_count as f64 / valid_count as f64) * 100.0
        } else {
            100.0
        };

        TransitionCoverageReport {
            state_machine: self.name.clone(),
            total_possible_transitions: total_possible,
            valid_transitions: valid_count,
            tested_transitions: tested_count,
            coverage_percentage: coverage,
            untested_transitions: untested,
            all_valid_transitions: self.valid_transitions.clone(),
        }
    }
}

pub fn generate_mermaid_diagram(state_machine: &StateMachine) -> String {
    let mut diagram = String::from("stateDiagram-v2\n");
    diagram.push_str("    [*] --> ");

    if let Some((_, to)) = state_machine.valid_transitions.first() {
        diagram.push_str(&format!("{}\n", to));
    }

    let mut seen_transitions = HashSet::new();
    for (from, to) in &state_machine.valid_transitions {
        let transition_key = format!("{} --> {}", from, to);
        if !seen_transitions.contains(&transition_key) {
            diagram.push_str(&format!("    {} --> {}\n", from, to));
            seen_transitions.insert(transition_key);
        }
    }

    for state in &state_machine.states {
        let is_terminal = !state_machine.valid_transitions.iter()
            .any(|(_, to)| to == state || 
                 state_machine.valid_transitions.iter().any(|(from, _)| from == state));
        if is_terminal && state_machine.valid_transitions.iter().any(|(_, to)| to == state) {
            diagram.push_str(&format!("    {} --> [*]\n", state));
        }
    }

    diagram
}

pub fn generate_markdown_report(reports: &[TransitionCoverageReport]) -> String {
    let mut md = String::from("# State Transition Coverage Analysis Report\n\n");
    
    for report in reports {
        md.push_str(&format!("## {}\n\n", report.state_machine));
        md.push_str(&format!("| Metric | Value |\n"));
        md.push_str(&format!("|--------|-------|\n"));
        md.push_str(&format!("| Total Possible Transitions | {} |\n", report.total_possible_transitions));
        md.push_str(&format!("| Valid Transitions | {} |\n", report.valid_transitions));
        md.push_str(&format!("| Tested Transitions | {} |\n", report.tested_transitions));
        md.push_str(&format!("| Coverage | {:.2}% |\n\n", report.coverage_percentage));

        if !report.untested_transitions.is_empty() {
            md.push_str("### Untested Transitions\n\n");
            for (from, to) in &report.untested_transitions {
                md.push_str(&format!("- `{}` → `{}`\n", from, to));
            }
            md.push_str("\n");
        } else {
            md.push_str("✅ All valid transitions are tested!\n\n");
        }
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escrow_state_machine_coverage() {
        let sm = StateMachine::new(
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

        let mut tested = HashSet::new();
        tested.insert(("Pending".to_string(), "Active".to_string()));
        tested.insert(("Active".to_string(), "Released".to_string()));
        tested.insert(("Active".to_string(), "Disputed".to_string()));
        tested.insert(("Disputed".to_string(), "Resolved".to_string()));
        tested.insert(("Pending".to_string(), "Refunded".to_string()));

        let report = sm.analyze_coverage(tested);

        assert_eq!(report.state_machine, "EscrowStatus");
        assert_eq!(report.valid_transitions, 7);
        assert_eq!(report.tested_transitions, 5);
        assert!((report.coverage_percentage - 71.42857142857143).abs() < 0.01);
        assert_eq!(report.untested_transitions.len(), 2);
    }

    #[test]
    fn test_mermaid_diagram_generation() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["State1".to_string(), "State2".to_string(), "State3".to_string()],
            vec![
                ("State1".to_string(), "State2".to_string()),
                ("State2".to_string(), "State3".to_string()),
            ],
        );

        let diagram = generate_mermaid_diagram(&sm);

        assert!(diagram.contains("stateDiagram-v2"));
        assert!(diagram.contains("[*] -->"));
        assert!(diagram.contains("State1 --> State2"));
        assert!(diagram.contains("State2 --> State3"));
    }

    #[test]
    fn test_markdown_report_generation() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["State1".to_string(), "State2".to_string()],
            vec![("State1".to_string(), "State2".to_string())],
        );

        let mut tested = HashSet::new();
        tested.insert(("State1".to_string(), "State2".to_string()));

        let report = sm.analyze_coverage(tested);
        let markdown = generate_markdown_report(&[report]);

        assert!(markdown.contains("# State Transition Coverage Analysis Report"));
        assert!(markdown.contains("TestStatus"));
        assert!(markdown.contains("Coverage"));
        assert!(markdown.contains("100.00%"));
    }

    #[test]
    fn test_zero_coverage() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["State1".to_string(), "State2".to_string()],
            vec![("State1".to_string(), "State2".to_string())],
        );

        let tested = HashSet::new();

        let report = sm.analyze_coverage(tested);

        assert_eq!(report.coverage_percentage, 0.0);
        assert_eq!(report.tested_transitions, 0);
        assert_eq!(report.untested_transitions.len(), 1);
    }

    #[test]
    fn test_perfect_coverage() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["State1".to_string(), "State2".to_string()],
            vec![("State1".to_string(), "State2".to_string())],
        );

        let mut tested = HashSet::new();
        tested.insert(("State1".to_string(), "State2".to_string()));

        let report = sm.analyze_coverage(tested);

        assert_eq!(report.coverage_percentage, 100.0);
        assert_eq!(report.tested_transitions, 1);
        assert_eq!(report.untested_transitions.len(), 0);
    }

    #[test]
    fn test_multiple_untested_transitions() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["S1".to_string(), "S2".to_string(), "S3".to_string(), "S4".to_string()],
            vec![
                ("S1".to_string(), "S2".to_string()),
                ("S2".to_string(), "S3".to_string()),
                ("S2".to_string(), "S4".to_string()),
                ("S3".to_string(), "S4".to_string()),
            ],
        );

        let mut tested = HashSet::new();
        tested.insert(("S1".to_string(), "S2".to_string()));

        let report = sm.analyze_coverage(tested);

        assert_eq!(report.valid_transitions, 4);
        assert_eq!(report.tested_transitions, 1);
        assert_eq!(report.untested_transitions.len(), 3);
        assert!((report.coverage_percentage - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_markdown_with_untested_transitions() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["S1".to_string(), "S2".to_string()],
            vec![
                ("S1".to_string(), "S2".to_string()),
            ],
        );

        let tested = HashSet::new();
        let report = sm.analyze_coverage(tested);
        let markdown = generate_markdown_report(&[report]);

        assert!(markdown.contains("Untested Transitions"));
        assert!(markdown.contains("S1"));
        assert!(markdown.contains("S2"));
    }

    #[test]
    fn test_markdown_with_perfect_coverage() {
        let sm = StateMachine::new(
            "TestStatus".to_string(),
            vec!["S1".to_string(), "S2".to_string()],
            vec![
                ("S1".to_string(), "S2".to_string()),
            ],
        );

        let mut tested = HashSet::new();
        tested.insert(("S1".to_string(), "S2".to_string()));

        let report = sm.analyze_coverage(tested);
        let markdown = generate_markdown_report(&[report]);

        assert!(markdown.contains("✅ All valid transitions are tested!"));
    }
}
