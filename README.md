# Spines

Spines is one self-hosted library for music and books. It scans the folders you name, stores the catalog in SQLite, and serves a web page and a read-only MCP server from the same process.

## Two folders

The data folder holds `spines.db`, the generated spine images, and the cover cache. Put it on the host, outside the container. Back up this folder. To move the app, copy this folder and point the new container at it. Recreating the container with the same folder brings the library back. Spines does not rescan when the file stamps still match.

The media folders are the libraries. Mount them read-only. Spines does not write them.

Set the paths with environment variables.

- `SPINES_DATA` is the data folder. The compose file mounts the host path `SPINES_DATA_DIR` at `/data`.
- `SPINES_MEDIA` is the read-only mount a music folder must stay inside. It defaults to `SPINES_MUSIC`.
- `SPINES_MUSIC` is the folder the first-open screen offers. Spines does not scan it until that choice is saved in the data folder. A missing saved folder leaves the albums already in the catalog. An empty saved folder is an empty shelf, and the next scan removes those albums.
- `SPINES_BOOKS` is the books root. It is a Calibre library. `metadata.db` can sit in that directory or one directory down. Spines opens it read-only. A missing database leaves the books already in the catalog.
- `SPINES_BIND` is the listen address. It defaults to `0.0.0.0:8787`. There are no accounts. Anyone who can reach this address can use the library.
- `SPINES_FORCE_RESCAN=1` reads every file on that start, even when the stamp matches.

## Run with Docker

Copy `docker-compose.example.yml` and set the three host paths.

```sh
export SPINES_DATA_DIR=/srv/spines
export SPINES_MUSIC_DIR=/srv/music
export SPINES_BOOKS_DIR=/srv/books
docker compose -f docker-compose.example.yml up --build
```

Open `http://127.0.0.1:8787`.

The first visit asks where the music is. The field starts with `SPINES_MUSIC`. Continue checks that the folder exists, is readable, and sits inside `SPINES_MEDIA`, then saves the path and scans. Library in the header changes it later. A changed folder scans only files whose size or time changed. With no `SPINES_MEDIA` and no `SPINES_MUSIC`, that screen stays closed and books still open.

Books are scanned at startup from `SPINES_BOOKS`. The log line looks like this.

```text
scan plugin=music changed=24 unchanged=0 removed=0 elapsed_ms=6138 skipped=false reason=reconciled
```

The next start, with the same data folder, skips the setup screen. When the file stamps still match, the log says `skipped=true reason=stamp` and Spines does not read tags or `metadata.db`.

`POST /api/rescan` scans again. `POST /api/rescan?force=true` reads every file.

## Shelves

Music is a row of CD jewel-case spines built from the album art and the tags. Tap a case. It slides out and plays in the browser. Previous and Next change the track.

Books stand on a plank. The color comes from the cover. The width and the type come from the Calibre row, using the same rules as the bookshelf app at `163a6445`. The page shows the title, the authors, and the formats. It does not read the book and it does not download the file. Calibre-Web still owns the files.

Search keeps an item only when every word is in its title, people, series, publisher, tags, or description.

Each shelf is a `Plugin` in `src/plugin.rs`. A plugin lists files, reads one changed file, and folds those reads into one item. The reconciler owns the stamp, the content hash, and the SQLite write. A new shelf is a new type plus one line in `registry`.

Music item ids are `mus_` plus a hash of the lowercased artist and album, so the id does not depend on the folder path. Book item ids are `bok_` plus the Calibre integer id.

## HTTP and MCP

`GET /openapi.json` is generated from the Rust types. The committed copy is `openapi/openapi.json`. `cargo test` fails if the two differ.

`POST /mcp` is JSON-RPC on the server, so it can report what the browser is playing. `spines mcp` speaks MCP on stdin. It opens the catalog read-only, it does not scan, and now-playing is empty because playback stays in the server process.

## Not in this version

No streaming to other players. No tag editing. No downloads. No replacement for Calibre.

## Demo

`docs/demo/demo.webm` uses the public-domain sample library. The first open offers `/media/music`. Continue scans and the albums appear. A terminal then runs `docker compose up -d --force-recreate` on the same data folder. The new container logs `skipped=true reason=stamp`, and the albums are already there.

## Umbrel

`umbrel/` is the app package. Books mount the Calibre-Web library read-only, and Calibre-Web is a dependency. Music mounts the Umbrel Downloads folder read-only at `/media`, and the first-open screen starts at `/media/music` (`Downloads/music`). The image reference is not a published image.
