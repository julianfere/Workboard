"""Test launcher: runs plugin.py's protocol loop with the fake Trello API instead of the network.

It lives under tests/ and is never referenced by the manifest, so production runs always use urllib.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.dirname(HERE))

import plugin  # noqa: E402
from fake_trello import FakeTrello  # noqa: E402

plugin.HTTP = FakeTrello()
plugin.main()
