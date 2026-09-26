# 固定源码依据

核查日期：2026-09-26。除分支与CI状态外均固定提交。

## Current branch metadata

https://api.github.com/repos/Furinelle/Mnemosyne/branches/master

a0da375; last commit 2026-09-20T14:13:50Z

## Current CI

https://github.com/Furinelle/Mnemosyne/actions/runs/35515845048

success at inspection; not reviewer rerun

## Cargo version

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/Cargo.toml

2.0.1; edition2024; MSRV1.95

## Repair evidence

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/docs/plans/native-evolution/EXECUTION_LOG.md

repair section from line385; 159 tests/15 gates are repository records

## Ingestion routing

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/src/ingest.rs

process at lines230ff uses V2 for upgraded stores

## Reconciliation

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/src/reconcile.rs

value comparison precedes same_body

## Provenance

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/src/provenance.rs

recorded support target and source revision binding

## Sleep

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/src/sleep.rs

paged body reads; inventory and output hard caps remain

## Scoring harness

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/tests/native_agent_eval.py

source of minimal scoring-expression counterexample

## Performance harness

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/tests/native_perf.py

phase timings exist; corpus initialized then legacy Markdown written, no store upgrade

## CI configuration

https://github.com/Furinelle/Mnemosyne/blob/a0da375a872001b57cb4f259492961de6edc2de6/.github/workflows/ci.yml

offline native/transport/model mocks and recall gates; new scorer unit tests should be added
