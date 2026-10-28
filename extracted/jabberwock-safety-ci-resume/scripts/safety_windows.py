#!/usr/bin/env python3
"""Run Windows safety items 1..12 and write target/safety-windows-report.json."""
import sys
from safety_report import main

if __name__ == "__main__":
    sys.exit(main(target_os="windows"))
