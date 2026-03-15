"""mnemo_eval — AI Memory System Evaluation Framework.

Benchmark any memory backend using the Mnemo eval harnesses.

Quick start::

    from mnemo_eval import MnemoBackend, run_pack

    backend = MnemoBackend("http://localhost:8080")
    results = run_pack(backend, "temporal")

Or via the CLI::

    python -m mnemo_eval --backend mnemo --base-url http://localhost:8080 --packs all
    mnemo-eval --backend mnemo --base-url http://localhost:8080 --packs temporal,longmem

See eval/README.md for full documentation.
"""

from __future__ import annotations

# Re-export the key public API so users can do:
#   from mnemo_eval import MemoryBackend, MnemoBackend, ZepBackend
import sys
from pathlib import Path

# When installed as a package, lib.py is a sibling module in the eval/ directory.
# Add the parent (eval/) to sys.path so relative imports work whether this package
# is installed via pip or run directly from the repo.
_EVAL_DIR = Path(__file__).parent.parent
if str(_EVAL_DIR) not in sys.path:
    sys.path.insert(0, str(_EVAL_DIR))

from lib import (  # noqa: E402
    EvalResultFile,
    HttpClient,
    MemoryBackend,
    MnemoBackend,
    ResultWriter,
    ZepBackend,
    p_quantile,
    print_table,
)

__version__ = "0.1.0"
__all__ = [
    "MemoryBackend",
    "MnemoBackend",
    "ZepBackend",
    "HttpClient",
    "ResultWriter",
    "EvalResultFile",
    "p_quantile",
    "print_table",
]
