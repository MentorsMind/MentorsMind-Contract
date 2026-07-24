# Diagram Assets

This directory contains architecture and state-machine PNG diagrams used by project docs.

## Files

- `state_machine.png`
- `system_architecture.png`
- `contract_relationships.png`
- `data_flow.png`
- `deployment_architecture.png`
- `*.mmd` Mermaid source files for the same diagrams

## Update Rule

When contract states, relationships, or deployment flow changes:
1. update `ARCHITECTURE.md` and `docs/STATE_MACHINE.md`
2. regenerate diagrams in this folder so visual docs stay consistent
