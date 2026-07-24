#!/usr/bin/env python3
import argparse
import asyncio
import json
import time
from statistics import mean
from pathlib import Path

import aiohttp


async def create_escrow(session, url, payload):
    t0 = time.perf_counter()
    async with session.post(url, json=payload) as resp:
        status = resp.status
        try:
            data = await resp.json()
        except Exception:
            data = await resp.text()
    t1 = time.perf_counter()
    return {"status": status, "resp": data, "latency_ms": (t1 - t0) * 1000}


async def run(args):
    create_url = args.endpoint.rstrip("/") + args.create_path
    query_url = args.endpoint.rstrip("/") + args.query_path if args.query_path else None

    results = []
    sem = asyncio.Semaphore(args.concurrency)

    async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=60)) as session:

        async def worker(i):
            async with sem:
                payload = args.payload_template.copy()
                # minimal unique fields
                payload[args.index_field] = i
                payload.setdefault("amount", "1token")
                payload.setdefault("memo", f"stress-{i}")
                return await create_escrow(session, create_url, payload)

        tasks = [asyncio.create_task(worker(i)) for i in range(args.start_index, args.start_index + args.count)]
        for coro in asyncio.as_completed(tasks):
            r = await coro
            results.append(r)

        # optional query for total count or storage
        query_result = None
        if query_url:
            try:
                async with session.get(query_url) as resp:
                    try:
                        query_result = await resp.json()
                    except Exception:
                        query_result = await resp.text()
            except Exception as e:
                query_result = {"error": str(e)}

    # Summarize
    latencies = [r["latency_ms"] for r in results if r.get("latency_ms")]
    statuses = {}
    gas_values = []
    for r in results:
        statuses[r["status"]] = statuses.get(r["status"], 0) + 1
        # try to capture gasUsed from response if present
        resp = r.get("resp")
        if isinstance(resp, dict):
            for key in ("gasUsed", "gas_used", "gas"):
                if key in resp:
                    try:
                        gas_values.append(int(resp[key]))
                    except Exception:
                        pass

    summary = {
        "total_requests": len(results),
        "statuses": statuses,
        "latency_ms": {"min": min(latencies) if latencies else None, "max": max(latencies) if latencies else None, "mean": mean(latencies) if latencies else None},
        "gas_samples_count": len(gas_values),
        "gas_samples_avg": mean(gas_values) if gas_values else None,
        "query_result": query_result,
    }

    out_dir = Path(args.output_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"stress_result_{int(time.time())}.json"
    with out_file.open("w") as f:
        json.dump({"params": vars(args), "results": results, "summary": summary}, f, indent=2, default=str)

    print(json.dumps(summary, indent=2))


def parse_args():
    p = argparse.ArgumentParser(description="Stress test runner for escrow creation endpoints")
    p.add_argument("--endpoint", required=True, help="Base HTTP endpoint, e.g. http://127.0.0.1:1317")
    p.add_argument("--create-path", default="/escrow/create", help="Path to POST create escrow")
    p.add_argument("--query-path", default="/escrow/list", help="Optional path to query escrows after run")
    p.add_argument("--count", type=int, default=10000, help="Number of escrows to create")
    p.add_argument("--concurrency", type=int, default=200, help="Number of concurrent requests")
    p.add_argument("--start-index", type=int, default=0, help="Start index for unique IDs")
    p.add_argument("--output-dir", default="tests/stress/results", help="Directory to write results")
    p.add_argument("--index-field", default="reference_id", help="Field in payload to make unique per request")
    p.add_argument("--payload-template", type=json.loads, default='{}', help="JSON string template for create payload")
    return p.parse_args()


def main():
    args = parse_args()
    # payload_template comes in as a dict via argparse conversion above
    # ensure it is a dict if a string default used
    if isinstance(args.payload_template, str):
        try:
            args.payload_template = json.loads(args.payload_template)
        except Exception:
            args.payload_template = {}
    asyncio.run(run(args))


if __name__ == "__main__":
    main()
