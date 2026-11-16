"""The on-demand brief engine, launched by the dashboard's two POST routes.

Split by stage so each piece has one job: `stages.py` (the three prompts and model calls),
`chain.py` (the default engine that runs them), `validate.py` (factual edit checks), `agent.py`
(the older Hermes path and its editorial review), `contacts.py` (the deep contact search),
`prompts.py` (template assembly), `posting.py` (the input row and text), `state.py` (the files
under `data/state/briefs/`), and `llm.py` (the DeepSeek client).

`generate.py` is the CLI entry point; `python3 -m server brief` reaches it through `cli.py`.
"""
