# Contributing

Shep is a native Rust + iced app. Start with the [installation guide](installation.md), then install the repository hooks:

```sh
bash scripts/install-hooks.sh
```

Run the app with `cargo run --release`. Run the complete checks with `bash scripts/check.sh`; native UI tests need Linux/X11 and the [harness dependencies](agents/development.md).

Use Conventional Commits, such as `fix: preserve the selected message`. Keep generated logs and test evidence under `artifacts/`.

## Edit these docs

From the checkout, using Python 3.12 or newer:

```sh
python3 -m venv .venv-docs
source .venv-docs/bin/activate
python -m pip install -r requirements-docs.txt
zensical serve
```

Open the local URL printed by Zensical. Run `zensical build --clean` to check the production build.

Keep user pages short and task focused. Put protocol details, invariants and test evidence in [agent docs](agents/index.md). The README is a brief introduction, not the manual.

## Publishing

The Documentation workflow builds pull requests and publishes docs changes on `main` to GitHub Pages. It uses the [Zensical Pages deployment flow](https://zensical.org/docs/publish-your-site/), with the dependency version pinned in `requirements-docs.txt`.

GitHub Pages must use **GitHub Actions** as its source. Private repositories need a GitHub plan that supports Pages.

Quality and release workflows remain disabled. Re-enable those when the self-hosted runners are ready.
