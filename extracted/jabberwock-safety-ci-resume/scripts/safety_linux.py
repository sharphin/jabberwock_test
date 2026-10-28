#!/usr/bin/env python3
"""Run Linux safety items 1..12 and write target/safety-linux-report.json."""
import sys
from safety_report import main

if __name__ == "__main__":
    sys.exit(main(target_os="linux"))
