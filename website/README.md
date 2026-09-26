# Shep website

The promotional site for shep.so. It is plain HTML, CSS and a little JavaScript, with no remote fonts, analytics or runtime dependencies. The only thing it stores is the chosen appearance.

The page picks the visitor's platform from the user agent and shows that install command first, with tabs for the others. The commands are the same one-liners as the [installation guide](https://sam-ruff.github.io/shep.so/installation/).

## The live demo

`/demo/` is the browser client from `web/` built in its preview mode, with the fictional mailbox from `shared/preview.json`. Its MIME and profile code is the shared Rust compiled to WebAssembly, and it runs entirely in the visitor's browser: it cannot send mail or reach a server. The home page only loads it when someone presses Start, because the WebAssembly is a few megabytes.

## Building

Build the demo once, then the site:

```sh
cd website
npm ci
npm run build:demo   # needs Rust with wasm32-unknown-unknown and wasm-bindgen-cli 0.2.128
npm run build
npm run preview
```

Open `http://127.0.0.1:4178`. Without a local Rust toolchain, Docker can build the demo instead:

```sh
docker build -f website/Dockerfile --target demo-files --output type=local,dest=web/dist-preview .
```

`SHEP_DEMO_DIR` points the build at a demo somewhere else.

## Tests

Install Python 3 with Pillow and the Playwright browsers, then:

```sh
npx playwright install chromium firefox webkit
npm test
npm run test:all-browsers
npm run test:deployment
```

The Playwright flows drive real controls: platform detection, tabs, Copy, the embedded demo, appearance, blocked storage and clipboard, no-JavaScript fallbacks and axe checks across phone to wide desktop sizes. Screenshots and reports go to the ignored `artifacts/website/`.

## Deployment

The image is built from the repository root with `docker build -f website/Dockerfile .`. A Rust stage compiles the WebAssembly, a Node stage builds the demo and the site, and Chainguard nginx serves the result on port 8080. Unknown paths return 404.

The `Website` workflow tests pull requests on the local runner pool and, on `main`, pushes `registry.tail2d6fbe.ts.net/shep/website:sha-<commit>`. Infrastructure pins that digest for staging and production.
