# record-maker

record-maker is a metadata-driven app builder. You describe tables, layouts, and logic; the runtime renders both a design surface (Layout Mode) and a live, data-bound app (Browse Mode) from that same metadata. If you've used FileMaker, the idea will feel familiar: design and run the app in the same place. This isn't a FileMaker clone and doesn't touch its file format, just an app built around a similar idea.

Eventually the plan is to let natural language drive the builder itself: describe the table, field, or layout you want, and an LLM issues typed edits against the metadata instead of writing freeform code. That keeps every change validated and undoable, since the engine is the one applying it, not the model.

## How it's put together

A solution is stored as two SQLite databases. `app.db` holds the metadata (tables, fields, layouts, relationships) in a versioned schema with a migration runner; `data.db` holds the actual user data in real tables that get created and altered as the metadata changes.

Layouts bind to a single table rather than routing through FileMaker's table-occurrence graph. Objects reach related data with a plain dot-path, like `invoice.bill_to.name`.

On the runtime side, a Rust engine (`crates/engine`) owns the metadata and data model, and everything else is built on top of it. An embedded axum server (`crates/server`) renders Browse Mode on the server with askama and HTMX. Layout Mode is a Svelte island (`ui/`), the one part of the app with real client-side interactivity, served by that same server and eventually wrapped in a Tauri 2 desktop shell. Because the desktop shell and the web target both run through that one server, there's no separate build for publishing to the web.

## Project layout

```
crates/engine/   metadata + data model, versioned schema, migrations
crates/server/   embedded axum server: Browse Mode runtime + web publishing
ui/              Layout Mode editor (Svelte 5 + Vite), built into ui/dist
src-tauri/       desktop shell (Tauri 2), not yet wired into the workspace build
scripts/         build/dev/run helpers
```

## Running it

```sh
scripts/dev.sh          # watch-builds ui/dist and runs the server at http://127.0.0.1:4317
scripts/build.sh        # one-off build of the UI bundle + Rust workspace
scripts/run.sh --server # headless server, no Tauri toolchain required
scripts/run.sh          # desktop app via Tauri, needs the Tauri v2 CLI and Linux system libs
```

If something's missing, Node for the UI bundle or the Tauri CLI and system libs for the desktop shell, each script says so and tells you how to install it.

## Status

Early days. The metadata contract and engine are further along than the editor and the natural-language layer. Architecture decisions and design notes live in the [wiki](https://github.com/solutionscay/record-maker/wiki), not in this repo.
