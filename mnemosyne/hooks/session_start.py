"""Backward-compat shim; the adapter lives in integrations/claude_code."""

from mnemosyne.integrations.claude_code.session_start import main

if __name__ == '__main__':
    main()
