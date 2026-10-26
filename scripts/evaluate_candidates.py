"""Compare a persistent trigram candidate prototype with bounded path scans.

This experiment is separate from the production index. Posting reads and body
checks have independent budgets; it does not claim equal total work for equal
body limits. Short terms fall back to a bounded path scan.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time


def grams(text):
    return {text[i:i+3] for i in range(max(0, len(text)-2))}


def query(database, text, limit, work_limit, engine='trigram'):
    if not isinstance(text, str) or not text.strip():
        raise ValueError('query must contain a non-whitespace term')
    if limit < 0 or work_limit < 0:
        raise ValueError('body and posting limits must be non-negative')
    terms = text.lower().split()
    posting_reads = 0
    complete = True
    with closing(sqlite3.connect(database)) as connection:
        tokens = sorted(set().union(*(grams(term) for term in terms))) if engine == 'trigram' else []
        if tokens:
            candidates = None
            for token in tokens:
                remaining = work_limit - posting_reads
                entries = connection.execute('SELECT path FROM postings WHERE gram=? ORDER BY path LIMIT ?',
                                             (token, remaining)).fetchall()
                if len(entries) == remaining:
                    complete = False
                posting_reads += len(entries)
                paths = {row[0] for row in entries}
                candidates = paths if candidates is None else candidates & paths
                if not complete or not candidates:
                    break
            paths = sorted(candidates or [])
        else:
            # The extra metadata row establishes whether body coverage is complete.
            paths = [row[0] for row in connection.execute('SELECT path FROM docs ORDER BY path LIMIT ?', (limit+1,))]
        if len(paths) > limit:
            complete = False
        found = []
        for path in paths[:limit]:
            raw, digest, normalized = connection.execute('SELECT raw,hash,normalized FROM docs WHERE path=?', (path,)).fetchone()
            assert hashlib.sha256(raw.encode()).hexdigest() == digest
            if all(term in normalized for term in terms):
                found.append(path)
    return {'matches': found, 'complete': complete, 'body_checks': min(len(paths), limit),
            'posting_rows': posting_reads, 'posting_limit': work_limit,
            'mode': 'trigram' if tokens else ('path-scan' if engine == 'scan' else 'short-term-scan')}


def query_process(database, output, text, limit, engine):
    started = time.perf_counter()
    response = json.loads(subprocess.check_output(
        [sys.executable, __file__, str(output), '--query-db', str(database),
         '--query', text, '--limit', str(limit), '--engine', engine], timeout=60))
    response['elapsed_ms_including_process_start'] = (time.perf_counter()-started)*1000
    return response


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--scale', type=int, default=1000)
    parser.add_argument('--query-db', type=Path)
    parser.add_argument('--query')
    parser.add_argument('--limit', type=int, default=10)
    parser.add_argument('--posting-limit', type=int, default=10000)
    parser.add_argument('--engine', choices=('scan','trigram'), default='trigram')
    args = parser.parse_args()
    if args.query_db:
        try:
            response = query(args.query_db, args.query, args.limit, args.posting_limit, args.engine)
        except ValueError as error:
            parser.error(str(error))
        print(json.dumps(response))
        return
    if not 1 <= args.scale <= 10000:
        parser.error('--scale must be 1..10000')
    docs = {f'a/{i:05}.md': ('ordinary background ' * 50, 'background') for i in range(args.scale)}
    docs.update({'z/tail.md': ('late-evidence customer_id 7.2.0 认证流程 İSTANBUL', 'selected'),
                 'z/duplicate.md': ('late-evidence with distinct provenance', 'selected')})
    tasks = ['late-evidence', 'ustomer', 'customer_id', '7.2.0', '认证流程', 'İSTANBUL', '认证', 'absent-evidence']
    with tempfile.TemporaryDirectory(prefix='okf-candidates-') as temporary:
        database = Path(temporary) / 'prototype.sqlite'
        started = time.perf_counter()
        with closing(sqlite3.connect(database)) as connection:
            connection.executescript('CREATE TABLE docs(path TEXT PRIMARY KEY,raw TEXT,hash TEXT,normalized TEXT,tag TEXT); CREATE TABLE postings(gram TEXT,path TEXT,PRIMARY KEY(gram,path)) WITHOUT ROWID; CREATE INDEX tags ON docs(tag,path);')
            for path, (raw, tag) in docs.items():
                connection.execute('INSERT INTO docs VALUES(?,?,?,?,?)', (path,raw,hashlib.sha256(raw.encode()).hexdigest(),raw.lower(),tag))
                connection.executemany('INSERT INTO postings VALUES(?,?)', ((g,path) for g in grams(raw.lower())))
            connection.commit()
            # Exact metadata filtering can use an indexed column before body reads.
            selected = [r[0] for r in connection.execute("SELECT path FROM docs WHERE tag='selected' ORDER BY path")]
            assert set(selected) == {'z/tail.md', 'z/duplicate.md'}
        build_ms = (time.perf_counter()-started)*1000
        runs = []
        for text in tasks:
            terms = text.lower().split()
            expected = sorted(p for p,(raw,_) in docs.items() if all(t in raw.lower() for t in terms))
            for limit in (1,10,len(docs)):
                response = query_process(database, args.output, text, limit, 'trigram')
                baseline = [p for p in sorted(docs)[:limit] if all(t in docs[p][0].lower() for t in terms)]
                scan = query_process(database, args.output, text, limit, 'scan')
                assert scan['matches'] == baseline
                assert set(response['matches']) <= set(expected)
                if response['complete']:
                    assert response['matches'] == expected
                if limit == len(docs):
                    assert response['complete'] and response['matches'] == expected
                runs.append({'query':text,'body_limit':limit,'expected':expected,'path_scan':scan,**response})
        limited = query(database, 'background', 10, 1)
        assert not limited['complete'] and limited['posting_rows'] <= 1
        report = {'prototype_only':True, 'documents':len(docs),'build_ms':build_ms,
                  'database_bytes':database.stat().st_size,'raw_bytes':sum(len(raw.encode()) for raw,_ in docs.values()),
                  'metadata_selected':selected,'posting_limit_case':limited,'runs':runs}
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f'Candidate prototype passed: {len(runs)} cases; build {build_ms:.1f} ms, size {report["database_bytes"]} bytes')


if __name__ == '__main__':
    main()
