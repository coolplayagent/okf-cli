"""Run schema, HTTPS Git, retrieval and packaged-binary acceptance for CI."""
from pathlib import Path
import argparse
import os
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--tag', default=os.environ.get('OKF_RELEASE_TAG') or None)
args = parser.parse_args()
binary = args.binary.resolve()
subprocess.run([sys.executable, str(root / 'scripts/check_support.py')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_schema.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_index.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_scope.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/evaluate_index_candidates.py'), str(binary), str(root / 'target/verification/index-candidates.json')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_upstream.py'), str(root / 'target/verification/upstream-local.json')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_package.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_git_https.py'), str(binary)], check=True)
subprocess.run([sys.executable, str(root / 'scripts/evaluate.py'), str(binary), str(root / 'target/verification/retrieval.json')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/evaluate_multi_bundle.py'), str(binary), str(root / 'target/verification/multi-bundle.json')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/evaluate_budgets.py'), str(binary), str(root / 'target/verification/budgets.json')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/evaluate_consumption.py'), str(binary), str(root / 'target/verification/consumption.json')], check=True)
subprocess.run([sys.executable, str(root / 'scripts/check_agent_preparation.py'), str(binary)], check=True)
package_args = ['--tag', args.tag] if args.tag else []
archive = subprocess.check_output([sys.executable, str(root / 'scripts/package.py'), str(binary), *package_args], text=True).strip()
subprocess.run([sys.executable, str(root / 'scripts/verify_package.py'), archive], check=True)
