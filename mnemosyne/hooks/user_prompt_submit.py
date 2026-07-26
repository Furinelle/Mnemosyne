"""Backward-compat shim; the adapter lives in integrations/claude_code."""

from mnemosyne.integrations.claude_code.user_prompt_submit import main

if __name__ == '__main__':
    main()
