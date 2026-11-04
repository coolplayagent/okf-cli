# CLI workflows

Read only the section needed for the current task. Examples use `okf` as shorthand for the executable resolved in SKILL.md; retain the user's project working directory and request `--format json` for business commands. Run `okf <command> --help` for the installed binary's supported options; help and `--version` produce text even with `--format json`.

## Retrieval and filters

Queries match all whitespace-separated terms as case-insensitive substrings, not semantic similarity. Metadata searches match field values (ID, title, description, type and tags); indexed queries also match body text. Start with a few discriminating terms. A complete zero-result response applies only to its selected scope and filters.

Use `bundle` for knowledge-package registration and `--bundle` for selection. OKF `sources` frontmatter describes a concept's provenance; the response envelope uses `bundles` and `bundle_id` for knowledge-package read observations.

After browsing the bundle map, use `--dir PATH` on search or queried context to restrict the request to a bundle-relative directory and its descendants. Totals and complete coverage then describe that directory only; zero matches do not establish absence elsewhere in the bundle.

Bindings can reference local directories or pinned Git commits. Git bindings keep their commit until an explicit `bundle refresh NAME`; preserve `upstream_revision` when citing them. Check command help for supported options. Default `context --query` returns metadata candidates and read entry points; fetch their bodies before using the knowledge.

Use `--type`, `--tag` and `--status` to filter search or queried context before result selection; values match exactly. Use `okf neighbors '@NAME/path' --direction incoming --format json` (choose `incoming`, `outgoing`, or `both`) after synchronization to inspect direct indexed relationships. Keep each edge's generation and evidence hash/location or source field. A missing target remains an edge; incomplete relationship coverage cannot prove that no incoming links exist. This CLI does not infer relationships from text similarity.

When the task requires review or expiry constraints, use `--trust human-reviewed` or `--expiry current`; trust choices are `unverified`, `machine-confirmed`, `human-reviewed`, and expiry choices are `current`, `expired`, `unspecified`, `unknown`. Repeat a field for alternatives; different fields combine with AND. `--as-of` accepts an RFC 3339 timestamp with an offset to reproduce lifecycle evaluation, including offline index queries. Preserve `data.evaluated_at`; it is distinct from the observation time and does not prove the source is current. Apply these constraints when relevant to the task rather than silently hiding all unreviewed or undated knowledge.

## Indexed and offline workflows

```sh
okf search "task terms" --bundle NAME --full-text --format json
okf context --query "task terms" --bundle NAME --content --format json
okf read '@NAME/path' --indexed --generation GENERATION --format json
```

For synchronized indexes, use `search "terms" --bundle NAME --full-text` for body snippets or `context --query "terms" --bundle NAME --content` for larger original excerpts. Repeat `--bundle` to query several bundles with one shared result, byte, scan and time budget. Scan shares are distributed by bundle name; increasing the number of bundles does not increase the budget. Multi-bundle responses provide `data.generations`; use each item's own `generation` for `read '@NAME/path' --indexed --generation GENERATION`. A missing or failed index leaves other bundles usable but makes coverage incomplete; inspect `data.coverage_gaps.unavailable_bundles` and per-bundle observations. Identical or conflicting text retains its separate sources.

`freshness: cached` describes a previous observation, even when the source is now offline; it does not prove the current source is unchanged. Keep `generation`, `content_sha256` and snippet byte ranges with each excerpt. Pass a returned cursor for later read pages. `INDEX_MISSING` means no complete index has been published. Explicit `sync NAME` scans and publishes a new index; a partial sync preserves the old one. Follow the user's request when deciding to refresh or synchronize.

Indexed search and context can opt into persistent candidate selection with `--candidate-budget N` (0..1000000). Candidate rows and body scans have separate shared budgets; inspect both and treat `CANDIDATE_LIMIT` as incomplete coverage. Default queries keep path scanning. `INDEX_CANDIDATES_MISSING` requires a new sync to build derived data; `INDEX_CANDIDATES_CORRUPT` requires integrity checking and repair. Follow the user's synchronization instructions, or use the default path scan when appropriate; never interpret an unavailable candidate index as no evidence.

When index integrity is in question, use `index verify NAME --format json` for a read-only check of stored paths, hashes and generation. Inspect `data.complete` and `data.valid`: a null validity result is inconclusive. A valid index does not prove that live source files are unchanged. Follow the user's request when deciding to repair findings with `sync NAME`.

Candidate data has an independent format version. Older derived data can require a new sync even when ordinary indexed reads still work. Do not treat this as a need to recreate bindings or delete databases. `--raw` and `--indexed` are mutually exclusive; `--generation` requires `--indexed`.

## Validation

Run validation when the task asks about format or knowledge quality. Do not make a full validation scan a prerequisite for ordinary retrieval.

```sh
okf validate NAME --format json
okf validate NAME --attachments --anchor-style headings-v1 --format json
okf validate NAME --max-warnings 0 --format json
```

- For `Attested Computation`, read the contract and its `# Computation` content or file declaration. Static validation reports definition issues; per-run receipt verification requires a separate execution and attestation workflow.
- Ordinary metadata and body warnings do not invalidate a concept. Diagnostic truncation can leave checks incomplete, and an explicit warning threshold can change the exit status. Use `path` with the JSON Pointer `field` or original-file byte `location` to locate findings. Preserve relevant qualifications when using the knowledge. `validate` checks link targets only after complete enumeration; inspect `data.link_check.complete` before interpreting the absence of broken-link warnings.
- `validate` also checks declared bundle-local computation resources without loading or executing them. Check `data.resource_check.complete` separately from Markdown link coverage; external URIs are counted as skipped.
- `validate --attachments` checks non-Markdown body links and image references via metadata queries without reading attachment content; `data.attachment_check` reports `images_included`, `images_checked`, and `types` (lowercase, deduplicated, ascending extensions of checked targets).
- `validate --anchor-style headings-v1` also matches explicit HTML `id`/`name` anchors, decoding character references (`&amp;`, `&#45;`, `&#x2D;`) first; undecodable fragments and unrecognized custom HTML fall into `data.anchor_check.skipped` instead of a false missing-anchor finding.
- With explicit `validate --max-warnings N`, inspect `data.quality_gate`: `warnings_observed` is a lower bound; `warnings_total` is null when counting is incomplete. `passed: null` is inconclusive. A crossed threshold produces exit 1 when checks completed; partial checks retain exit 7. This policy does not change the format's conformance requirements.

## Recovery and runtime state

Exit codes: 0 success; 1 validation findings; 2 arguments/configuration; 3 missing bundle/document; 4 denied access; 5 I/O, cache/config lock failure, or index-integrity corruption found by a complete `index verify`; 6 changed revision; 7 partial result, resource/scan/time limit, deadline or cancellation. Exit 7 can also be a zero-result `status: error` response (for example `OUTPUT_TOO_LARGE`, `SCAN_LIMIT` while rebuilding a cursor listing, `DOCUMENT_TOO_LARGE` or `LFS_CONTENT_UNAVAILABLE`), with no usable results: read `status` and `data.error.code` before assuming partial results are available. Partial results may be useful only with their stated limitations.

- `CARD_TOO_LARGE` identifies a skipped card; increase the content budget or read its path directly. Other cards remain usable.
- `CANCELLED` or `DEADLINE_EXCEEDED`: use completed results with their stated limits. Do not automatically retry a user cancellation. Consumption commands accept `--timeout-ms N`, defaulting to 30000, and cooperate with Ctrl+C.
- `SCAN_LIMIT` on browse: increase `--max-scan` and restart. An incomplete directory listing has no continuation cursor.
- `CACHE_WRITE_FAILED` or `CACHE_UNAVAILABLE`: readable knowledge can still be returned. Cache diagnostics do not by themselves imply incomplete knowledge coverage or override complete validation findings.

`context` can be limited by the number of bundles or fail while probing live sources. Use `okf bundle list --format json` to inspect configured bindings; select `--bundle NAME` explicitly, and add `--scope project` or `--scope user` when aliases are ambiguous. An offline indexed bundle can remain usable even if live probing fails.

For `RESULT_LIMIT`, `BUDGET_EXCEEDED` or `OUTPUT_TOO_LARGE`, narrow the scope or adjust the relevant result/content budget within command help limits. Search has no continuation cursor. Do not repeatedly retry an unchanged failing request; report the unavailable scope when it prevents the task.

Knowledge source files remain read-only, but ordinary reads can create/update the observation cache under the runtime home. `sync` publishes a content index; `bundle refresh` can fetch remote objects and changes the pinned commit. These operations must follow the user's task and existing authorization. Missing bindings require a known source path; create a binding only when setup is authorized, otherwise explain what is missing. Do not reset caches, clear indexes, run GC, or install dependencies merely to answer a knowledge question.
