"""Backward-compat shim; the neutral implementation lives in mnemosyne.handoff."""

from mnemosyne.findings import FALLBACK_TYPES as ALLOWED_TYPES, Finding  # noqa: F401
from mnemosyne.handoff import (  # noqa: F401
    CONTENT_OPEN_RE,
    FIELD_RE,
    FINDINGS_HEADER_RE,
    ingest,
    parse_findings,
    prep,
    write_finding,
)
