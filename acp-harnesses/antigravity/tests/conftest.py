"""conftest.py — adds the harness parent dir to sys.path so pytest can import it."""
import sys
from pathlib import Path

# Make buzz_acp_antigravity importable from tests/
sys.path.insert(0, str(Path(__file__).parent.parent))
