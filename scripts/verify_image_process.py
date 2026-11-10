#!/usr/bin/env python3
"""Run the narrow image OS-process case; prepare by default, never build.

See tests/model_process/image_http_case.py --help. Execution requires the caller's
explicit serial PG window and a new private evidence directory outside this repo.
"""
from pathlib import Path
import runpy
import sys
sys.dont_write_bytecode = True
runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tests/model_process/image_http_case.py'), run_name='__main__')
