"""Run schema, HTTPS Git, retrieval and packaged-binary acceptance for CI."""
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
subprocess.run([sys.executable, str(root / 'scripts/check_schema.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_git_https.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/evaluate.py'), str(binary), str(root / 'target/verification/retrieval.json')], check=True)
archive = subprocess.check_output([sys.executable, str(root / 'scripts/package.py'), str(binary)], text=True).strip()
subprocess.run([sys.executable, str(root / 'scripts/verify_package.py'), archive], check=True)
