#!/usr/bin/env python3
"""Audit each unique ranked domain in an isolated, time-bounded process."""
import argparse
import concurrent.futures
import datetime
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/compatibility-report.json")
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=4)
    parser.add_argument("--filter", default="", help="Only domains containing this text")
    args = parser.parse_args()
    targets = {}
    for line in (ROOT / "tests/fixtures/compatibility/sites.tsv").read_text().splitlines():
        if line.startswith("#") or line.startswith("region"):
            continue
        region, rank, domain = line.split("\t")
        if args.filter in domain:
            targets.setdefault(domain, {})[region] = int(rank)

    def inspect(domain):
        try:
            result = subprocess.run(
                [str(ROOT / "target/debug/examples/site-report"), f"https://{domain}/"],
                capture_output=True, text=True, timeout=65, check=True,
            )
            report = json.loads(result.stdout)
        except subprocess.TimeoutExpired:
            report = {"error": "Audit process exceeded 65 seconds"}
        except (subprocess.CalledProcessError, json.JSONDecodeError) as error:
            report = {"error": str(error)}
        report.update(domain=domain, ranks=targets[domain])
        return report

    reports = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(inspect, domain) for domain in targets]
        for future in concurrent.futures.as_completed(futures):
            report = future.result()
            reports.append(report)
            print(f"{len(reports)}/{len(targets)} {report['domain']}: "
                  f"{report.get('error', report.get('http_status'))}", flush=True)
    reports.sort(key=lambda r: r["domain"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({
        "checked_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "ranking_month": "2026-08", "scripts_executed": False,
        "note": "HTTP and static CSS audit; a parsed response is not proof of functional website support.",
        "sites": reports,
    }, ensure_ascii=False, indent=2) + "\n")
    print(f"Saved {len(reports)} results to {args.output}")


if __name__ == "__main__":
    main()
