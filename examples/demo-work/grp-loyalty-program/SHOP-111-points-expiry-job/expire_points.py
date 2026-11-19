"""Expire loyalty points older than 12 months (dry run by default).

Usage: python expire_points.py [--apply]
"""

import sys

DRY_RUN = "--apply" not in sys.argv

if __name__ == "__main__":
    print("dry run" if DRY_RUN else "applying", "- nothing to do in the demo")
