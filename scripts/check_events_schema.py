#!/usr/bin/env python3
"""Verify that every event symbol defined in shared/src/events.rs is
documented in events_schema.json at the workspace root.

events.rs groups `evt_*` helpers under section headers of the form

    // --- <contract> ---
    // --- <contract> (<human readable name>) ---

where <contract> is the contract symbol returned by the matching
`contract_*` helper (and the key used under `contracts` in the schema).

The check fails when:
  * a section header names a contract symbol not defined by a `contract_*` helper,
  * an `evt_*` helper appears outside a section,
  * a contract section has no entry in events_schema.json,
  * an event symbol has no entry under its contract in events_schema.json,
  * a schema entry is malformed (see the "format" key in events_schema.json).

Schema entries with no matching `evt_*` helper are reported as warnings only:
several contracts still publish events directly without a shared helper.

Usage: python3 scripts/check_events_schema.py [--events PATH] [--schema PATH]
"""

import argparse
import json
import re
import sys

DEFAULT_EVENTS = "contracts/shared/src/events.rs"
DEFAULT_SCHEMA = "events_schema.json"

CONTRACT_FN_RE = re.compile(
    r'pub\s+fn\s+contract_\w+\s*\(\s*env\s*:\s*&Env\s*\)\s*->\s*Symbol\s*'
    r'\{\s*Symbol::new\(\s*env\s*,\s*"([^"]+)"\s*\)\s*\}'
)
EVENT_FN_RE = re.compile(
    r'pub\s+fn\s+(evt_\w+)\s*\(\s*env\s*:\s*&Env\s*\)\s*->\s*Symbol\s*'
    r'\{\s*Symbol::new\(\s*env\s*,\s*"([^"]+)"\s*\)\s*\}'
)
SECTION_RE = re.compile(r'^//\s*---\s*([a-z0-9_]+)\b')


def error(path, line, msg):
    loc = f"file={path},line={line}" if line else f"file={path}"
    print(f"::error {loc}::{msg}")


def warning(path, msg):
    print(f"::warning file={path}::{msg}")


def parse_events(path):
    """Return (contract_symbols, [(contract, fn_name, event_symbol, line)], errors)."""
    with open(path, "r", encoding="utf-8") as f:
        lines = f.readlines()

    source = "".join(lines)
    contracts = set(CONTRACT_FN_RE.findall(source))

    events = []
    errors = 0
    section = None
    for lineno, line in enumerate(lines, start=1):
        m = SECTION_RE.match(line.strip())
        if m:
            section = m.group(1)
            if section not in contracts:
                error(path, lineno,
                      f"Section '{section}' is not a contract symbol defined by a "
                      f"contract_* helper in {path}")
                errors += 1
            continue

        m = EVENT_FN_RE.search(line)
        if m:
            fn_name, symbol = m.group(1), m.group(2)
            if section is None:
                error(path, lineno,
                      f"{fn_name} is not under a '// --- <contract> ---' section header")
                errors += 1
                continue
            events.append((section, fn_name, symbol, lineno))

    return contracts, events, errors


def validate_schema_shape(schema, path):
    errors = 0
    contracts = schema.get("contracts")
    if not isinstance(contracts, dict):
        error(path, None, "'contracts' must be an object keyed by contract symbol")
        return 1

    for contract, entry in contracts.items():
        events = entry.get("events") if isinstance(entry, dict) else None
        if not isinstance(events, dict):
            error(path, None, f"contracts.{contract}.events must be an object")
            errors += 1
            continue
        for name, evt in events.items():
            where = f"contracts.{contract}.events.{name}"
            if not isinstance(evt, dict):
                error(path, None, f"{where} must be an object")
                errors += 1
                continue
            if not isinstance(evt.get("description"), str) or not evt["description"]:
                error(path, None, f"{where}.description must be a non-empty string")
                errors += 1
            if "version" in evt and not isinstance(evt["version"], int):
                error(path, None, f"{where}.version must be an integer")
                errors += 1
            if "fields" in evt:
                fields = evt["fields"]
                if not isinstance(fields, list) or not all(
                    isinstance(fd, dict)
                    and isinstance(fd.get("name"), str)
                    and isinstance(fd.get("type"), str)
                    for fd in fields
                ):
                    error(path, None,
                          f"{where}.fields must be a list of {{\"name\", \"type\"}} objects")
                    errors += 1
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--events", default=DEFAULT_EVENTS)
    parser.add_argument("--schema", default=DEFAULT_SCHEMA)
    args = parser.parse_args()

    try:
        with open(args.schema, "r", encoding="utf-8") as f:
            schema = json.load(f)
    except (OSError, json.JSONDecodeError) as exc:
        error(args.schema, None, f"Cannot load schema: {exc}")
        return 1

    errors = validate_schema_shape(schema, args.schema)
    _, events, parse_errors = parse_events(args.events)
    errors += parse_errors

    schema_contracts = schema.get("contracts", {}) if isinstance(schema.get("contracts"), dict) else {}
    documented = set()
    for contract, fn_name, symbol, lineno in events:
        entry = schema_contracts.get(contract)
        if entry is None:
            error(args.events, lineno,
                  f"Contract '{contract}' ({fn_name}) has no entry in {args.schema}")
            errors += 1
            continue
        if symbol not in entry.get("events", {}):
            error(args.events, lineno,
                  f"Event '{contract}.{symbol}' ({fn_name}) is missing from {args.schema}; "
                  f"add it under contracts.{contract}.events")
            errors += 1
            continue
        documented.add((contract, symbol))

    for contract, entry in schema_contracts.items():
        for symbol in entry.get("events", {}) if isinstance(entry, dict) else {}:
            if (contract, symbol) not in documented:
                warning(args.schema,
                        f"'{contract}.{symbol}' has no evt_* helper in {args.events} "
                        f"(emitted directly by the contract?)")

    if errors:
        print(f"\nevents schema check FAILED with {errors} error(s).")
        return 1

    print(f"events schema check passed: {len(events)} event helper(s) documented.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
