"""Real Git HTTPS failure tests against an ephemeral trusted localhost server.

No public network is needed. The generated CA and key exist only in a temp directory.
"""
import datetime
import argparse
import base64
import http.server
import ipaddress
import json
import os
from pathlib import Path
import ssl
import shutil
import shlex
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import traceback
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--repeat', type=int, default=1)
parser.add_argument('--payload-bytes', type=int, default=32768)
parser.add_argument('--report-dir', type=Path, default=Path('target/verification/https'))
args = parser.parse_args()
if not 1 <= args.repeat <= 100:
    parser.error('--repeat must be between 1 and 100')
if not 32768 <= args.payload_bytes <= 1048576:
    parser.error('--payload-bytes must be between 32768 and 1048576')
binary = args.binary.resolve()
reports = args.report_dir.resolve()
reports.mkdir(parents=True, exist_ok=True)
if args.repeat > 1:
    for attempt in range(args.repeat):
        print(f'HTTPS regression round {attempt + 1}/{args.repeat}', flush=True)
        subprocess.run([sys.executable, __file__, str(binary), '--report-dir', str(reports / f'round-{attempt + 1}'),
                        '--payload-bytes', str(args.payload_bytes)], check=True)
    raise SystemExit(0)
git = os.environ.get('OKF_GIT', 'git')
with tempfile.TemporaryDirectory(prefix='okf-https-') as temporary:
    root = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(root / 'home'), NO_PROXY='localhost,127.0.0.1', no_proxy='localhost,127.0.0.1')
    for key in ('GIT_SSL_NO_VERIFY', 'GIT_CONFIG_COUNT', 'GIT_CONFIG_PARAMETERS'):
        env.pop(key, None)
    # The production adapter removes ambient GIT_* variables. A fixture-only
    # wrapper enables traces for localhost without changing production logging.
    if os.name != 'nt':
        executable = shutil.which(git) or git
        wrapper = root / 'trace-git'
        wrapper.write_text('#!/bin/sh\n' +
                           f'export GIT_TRACE2_EVENT={shlex.quote(str(root / "git-events.jsonl"))}\n' +
                           f'export GIT_TRACE_CURL={shlex.quote(str(root / "git-curl.log"))}\n' +
                           'export GIT_TRACE_CURL_NO_DATA=1\n' +
                           f'exec {shlex.quote(executable)} "$@"\n', encoding='utf-8')
        wrapper.chmod(0o755)
        env['OKF_GIT'] = str(wrapper)
    events = []
    events_lock = threading.Lock()
    phase = 'setup'
    def record(event, **fields):
        with events_lock:
            events.append({'time': time.monotonic(), 'phase': phase, 'event': event, **fields})
    payload_bytes = args.payload_bytes
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'OKF temporary localhost fixture')])
    now = datetime.datetime.now(datetime.timezone.utc)
    cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key())
            .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(minutes=5))
            .not_valid_after(now + datetime.timedelta(days=1))
            .add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost'), x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), critical=False)
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True).sign(key, hashes.SHA256()))
    (root / 'cert.pem').write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    (root / 'key.pem').write_bytes(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    mode = 401
    stopped = threading.Event()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            record('request', method=self.command, path=self.path, mode=mode)
            if mode == 'smart':
                parsed = urllib.parse.urlsplit(self.path)
                body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                upload_env = env.copy()
                upload_env.pop('GIT_PROTOCOL', None)
                command = [git, 'upload-pack', '--stateless-rpc']
                if self.command == 'GET':
                    assert parsed.path == '/repo.git/info/refs'
                    command += ['--advertise-refs']
                else:
                    assert parsed.path == '/repo.git/git-upload-pack'
                result = subprocess.run([*command, str(root / 'serve/repo.git')], input=body, env=upload_env, capture_output=True, timeout=10)
                if result.returncode:
                    record('upload_pack_error', code=result.returncode, stderr=result.stderr.decode(errors='replace'))
                result.check_returncode()
                content = result.stdout
                if self.command == 'GET':
                    banner = b'# service=git-upload-pack\n'
                    content = f'{len(banner) + 4:04x}'.encode() + banner + b'0000' + content
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-git-upload-pack-' + ('advertisement' if self.command == 'GET' else 'result'))
                self.send_header('Content-Length', str(len(content)))
                self.end_headers()
                self.wfile.write(content)
                record('response', bytes=len(content))
                return
            if mode == 'stall':
                stopped.wait(5)
                return
            if mode == 'disconnect':
                self.close_connection = True
                return
            self.send_response(mode)
            self.send_header('Content-Length', '0')
            self.end_headers()

        do_POST = do_GET

    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(root / 'cert.pem', root / 'key.pem')
    class TLSServer(http.server.ThreadingHTTPServer):
        def handle_error(self, request, client_address):
            record('server_error', detail=traceback.format_exc())

        def finish_request(self, request, client_address):
            # Handshake in the worker, not accept(): cancelled Git connections
            # must not block the listener and stall later fault-test requests.
            request.settimeout(10)
            try:
                connection = tls.wrap_socket(request, server_side=True)
            except (ssl.SSLError, ConnectionError, TimeoutError) as error:
                record('handshake_error', detail=str(error))
                return
            with connection:
                self.RequestHandlerClass(connection, client_address, self)

    server = TLSServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f'https://localhost:{server.server_port}/repo.git'
    add = ['bundle', 'add', 'remote', '--kind', 'git', '--url', url]

    def run(args, expected, code=5):
        global phase
        phase = expected
        result = subprocess.run([str(binary), '--format', 'json', *args], cwd=root, env=env, capture_output=True, timeout=15)
        response = json.loads(result.stdout)
        record('cli', arguments=args, code=result.returncode, response=response)
        assert result.returncode == code, (args, result.returncode, response)
        assert response['data']['error']['code'] == expected, response
        assert not result.stderr
        return response

    try:
        run(add, 'GIT_TLS_FAILED')
        cache = next((root / 'home/git').iterdir())
        def config(key, value):
            subprocess.run([git, '-C', str(cache), 'config', '--local', key, value], env=env, check=True, capture_output=True)
        config('http.sslCAInfo', str(root / 'cert.pem'))
        config('http.sslVerify', 'true')
        if os.name == 'nt':
            config('http.schannelUseSSLCAInfo', 'true')
        for mode, expected in [(401, 'GIT_AUTH_FAILED'), (403, 'GIT_AUTH_FAILED'), (404, 'GIT_REMOTE_NOT_FOUND'), ('disconnect', 'GIT_TRANSPORT_FAILED')]:
            run(add, expected)
        mode = 'stall'
        start = time.monotonic()
        run(add + ['--timeout-ms', '500'], 'DEADLINE_EXCEEDED', 7)
        assert time.monotonic() - start < 10
        result = subprocess.run([str(binary), '--format', 'json', 'bundle', 'list'], cwd=root, env=env, capture_output=True, check=True)
        assert json.loads(result.stdout)['data']['items'] == []
        # Serve real upload-pack responses through smart HTTP over the same TLS server.
        source = root / 'source'
        source.mkdir()
        remote = root / 'serve/repo.git'
        remote.parent.mkdir()
        def git_run(*args):
            return subprocess.run([git, '-C', str(source), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                                   '-c', 'commit.gpgsign=false', *args], env=env, capture_output=True, check=True)
        git_run('init', '--initial-branch=main')
        git_run('init', '--bare', '--initial-branch=main', str(remote))
        raw = '---\ntype: Note\n---\nHTTPS snapshot\n'
        (source / 'note.md').write_text(raw, encoding='utf-8')
        git_run('add', '.')
        git_run('commit', '-m', 'first')
        git_run('push', str(remote), 'main')
        mode = 'smart'
        def success(*args):
            global phase
            phase = ' '.join(args[:2])
            result = subprocess.run([str(binary), '--format', 'json', *args], cwd=root, env=env, capture_output=True, timeout=30)
            record('cli', arguments=args, code=result.returncode, stdout=result.stdout.decode(errors='replace'), stderr=result.stderr.decode(errors='replace'))
            assert result.returncode == 0, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)
        first = success(*add)['data']['bundle']['git']['commit']
        assert success('read', '@remote/note')['data']['document']['text'].replace('\r\n', '\n') == raw
        (source / 'note.md').write_text(raw + 'updated\n' + base64.b64encode(os.urandom(payload_bytes)).decode(), encoding='utf-8')
        git_run('add', '.')
        git_run('commit', '-m', 'second')
        git_run('push', str(remote), 'main')
        for mode, expected in [(403, 'GIT_AUTH_FAILED'), ('disconnect', 'GIT_TRANSPORT_FAILED')]:
            run(['bundle', 'refresh', 'remote'], expected)
            assert success('bundle', 'show', 'remote')['data']['bundle']['git']['commit'] == first
        mode = 'smart'
        run(['bundle', 'refresh', 'remote', '--max-git-bytes', '1'], 'GIT_STORAGE_LIMIT', 7)
        assert success('bundle', 'show', 'remote')['data']['bundle']['git']['commit'] == first
        quota = success('bundle', 'cache', 'remote')['data']['object_bytes'] + 1024
        run(['bundle', 'refresh', 'remote', '--max-git-bytes', str(quota)], 'GIT_STORAGE_LIMIT', 7)
        locks = [str(p.relative_to(cache)) for p in cache.rglob('*.lock')]
        record('after_quota', locks=locks)
        assert not locks, locks
        assert success('bundle', 'show', 'remote')['data']['bundle']['git']['commit'] == first
        assert success('bundle', 'refresh', 'remote')['data']['bundle']['git']['commit'] != first
        success('bundle', 'cache', 'remote', '--compact')
        assert 'updated' in success('read', '@remote/note', '--budget-bytes', '1024')['data']['document']['text']
    finally:
        stopped.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        (reports / 'server-events.json').write_text(json.dumps(events, indent=2) + '\n', encoding='utf-8')
        for name in ('git-events.jsonl', 'git-curl.log'):
            if (root / name).exists():
                shutil.copyfile(root / name, reports / name)
print('HTTPS fetch/refresh/compact and TLS/auth/not-found/disconnect/deadline/quota failures verified with real Git')
