"""Measure one CLI process, including startup, without pipe backpressure."""
import subprocess
import tempfile
import time

import psutil


def run_measured(command, *, cwd, env, timeout=120):
    start = time.perf_counter()
    peak = 0
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=stdout, stderr=stderr)
        try:
            try:
                observed = psutil.Process(process.pid)
            except psutil.NoSuchProcess:
                observed = None
            while process.poll() is None:
                try:
                    if observed is not None:
                        peak = max(peak, observed.memory_info().rss)
                except psutil.NoSuchProcess:
                    pass
                if time.perf_counter() - start > timeout:
                    raise TimeoutError(command)
                time.sleep(0.002)
        except BaseException:
            process.kill()
            process.wait()
            raise
        stdout.seek(0)
        stderr.seek(0)
        result = subprocess.CompletedProcess(command, process.returncode, stdout.read(), stderr.read())
    return result, (time.perf_counter() - start) * 1000, peak
