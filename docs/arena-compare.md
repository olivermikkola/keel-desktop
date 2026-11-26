# Arena offline compare (Desktop embed)

Keel Desktop can open a local [Keel Arena](https://github.com/olivermikkola/keel-arena) results JSON and show the offline compare shell inside the app.

## Boundary

| Owner | Responsibility |
| --- | --- |
| **Desktop (host)** | Sidebar **Arena** view, file picker, Compare / Dashboard / Overview tabs, iframe chrome |
| **Arena** | `load` / `summarize` / `write_cards` / `view` — validation, metrics, HTML cards |

Desktop shells out to:

```sh
python3 -m keel_arena view <results.json> --out <temp-dir>
```

then loads `index.html`, `cards/dashboard.html`, and `cards/overview.svg` from that temp directory. Nothing private is copied into the Desktop git tree. There are no model calls and no verifier bodies.

## Try it

1. Install Arena so `python3 -m keel_arena` works (`pip install -e .` from the Arena checkout).
2. Build and run Desktop from this repo (`cargo tauri dev` or your usual installer build).
3. In the sidebar, open **Arena**.
4. Choose a results JSON — public demo: Arena’s `examples/synthetic-gauntlet-results.json` (`purpose: demonstration`).
5. Click **Open compare**. Use the **Compare**, **Dashboard**, and **Overview** tabs.

Equivalent CLI proof (same Arena contract Desktop calls):

```sh
python3 -m keel_arena view examples/synthetic-gauntlet-results.json --out /tmp/arena-desktop-proof
```
