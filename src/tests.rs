use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use filetime::{set_file_mtime, FileTime};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use crate::domain::{ItemBody, PluginId};
use crate::http::{router, AppState};
use crate::mcp::{self, Reply};
use crate::{reconcile, scan, Config, Store};

fn wav(path: &Path) {
    let samples = 800u32;
    let data_len = samples * 2;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_len).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend((8000u32 * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_len.to_le_bytes());
    for i in 0..samples {
        let sample = ((i as f32 * 0.2).sin() * 2000.0) as i16;
        bytes.extend(sample.to_le_bytes());
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn sha(path: &Path) -> String {
    hex::encode(Sha256::digest(fs::read(path).unwrap()))
}

fn music_tree(root: &Path) -> (PathBuf, PathBuf) {
    let first = root.join("Artist/Album/01-prelude.wav");
    let second = root.join("Artist/Album/02-waltz.wav");
    wav(&first);
    wav(&second);
    (first, second)
}

fn calibre(root: &Path, nested: bool) {
    let script = r#"
import os, sqlite3, sys
root = sys.argv[1]
nested = sys.argv[2] == "1"
base = os.path.join(root, "Library") if nested else root
os.makedirs(base, exist_ok=True)
con = sqlite3.connect(os.path.join(base, "metadata.db"))
con.executescript('''
CREATE TABLE books (
  id INTEGER PRIMARY KEY, title TEXT, sort TEXT, author_sort TEXT,
  pubdate TEXT, series_index REAL, path TEXT, has_cover INTEGER
);
CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_authors_link (id INTEGER PRIMARY KEY, book INTEGER, author INTEGER);
CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_tags_link (id INTEGER PRIMARY KEY, book INTEGER, tag INTEGER);
CREATE TABLE data (id INTEGER PRIMARY KEY, book INTEGER, format TEXT);
CREATE TABLE publishers (id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_publishers_link (id INTEGER PRIMARY KEY, book INTEGER, publisher INTEGER);
CREATE TABLE series (id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_series_link (id INTEGER PRIMARY KEY, book INTEGER, series INTEGER);
CREATE TABLE comments (id INTEGER PRIMARY KEY, book INTEGER, text TEXT);
''')
con.execute("INSERT INTO books VALUES (1, 'Persuasion', 'Persuasion', 'Austen, Jane', '1817-12-01 00:00:00+00:00', 1.0, 'Austen/Persuasion', 0)")
con.execute("INSERT INTO books VALUES (2, 'Undated', 'Undated', '', '0101-01-01 00:00:00+00:00', 4.0, 'Nobody/Undated', 0)")
con.execute("INSERT INTO books VALUES (3, 'Escape', 'Escape', 'Tester', '1990-01-01 00:00:00+00:00', NULL, '../escape', 1)")
con.execute("INSERT INTO authors VALUES (1, 'Jane Austen'), (2, 'A. N. Other'), (3, 'Unknown'), (4, 'Tester')")
con.execute("INSERT INTO books_authors_link VALUES (2, 1, 2)")
con.execute("INSERT INTO books_authors_link VALUES (1, 1, 1)")
con.execute("INSERT INTO books_authors_link VALUES (3, 2, 3)")
con.execute("INSERT INTO books_authors_link VALUES (4, 3, 4)")
con.execute("INSERT INTO tags VALUES (1, 'Classic'), (2, 'England')")
con.execute("INSERT INTO books_tags_link VALUES (2, 1, 2)")
con.execute("INSERT INTO books_tags_link VALUES (1, 1, 1)")
con.execute("INSERT INTO data VALUES (1, 1, 'epub')")
con.execute("INSERT INTO publishers VALUES (1, 'John Murray')")
con.execute("INSERT INTO books_publishers_link VALUES (1, 1, 1)")
con.execute("INSERT INTO series VALUES (1, 'Novels')")
con.execute("INSERT INTO books_series_link VALUES (1, 1, 1)")
con.execute("INSERT INTO comments VALUES (1, 1, '<p>Anne &amp; Wentworth</p>')")
con.commit()
"#;
    let status = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(root)
        .arg(if nested { "1" } else { "0" })
        .status()
        .unwrap();
    assert!(status.success());
}

fn open_at(data: &Path, music: Option<&Path>, books: Option<&Path>) -> (Config, Store) {
    let cfg = Config::for_roots(
        data.to_path_buf(),
        music.map(Path::to_path_buf),
        books.map(Path::to_path_buf),
    );
    let store = Store::open(data).unwrap();
    (cfg, store)
}

#[test]
fn music_groups_an_album_and_the_second_pass_skips() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    music_tree(root.path());
    let (cfg, store) = open_at(data.path(), Some(root.path()), None);
    let reports = scan(&store, &cfg, false).unwrap();
    let music = reports.iter().find(|r| r.plugin == "music").unwrap();
    assert_eq!(music.changed, 2);
    assert!(!music.skipped);
    let items = store.items(Some(PluginId::Music), &[]).unwrap();
    assert_eq!(items.len(), 1);
    let ItemBody::Album(album) = &items[0].body else {
        panic!("album")
    };
    assert_eq!(album.artist, "Artist");
    assert_eq!(items[0].title, "Album");
    assert_eq!(album.tracks.len(), 2);
    assert!(items[0].id.starts_with("mus_"));
    let again = scan(&store, &cfg, false).unwrap();
    let music = again.iter().find(|r| r.plugin == "music").unwrap();
    assert!(music.skipped);
    assert_eq!(music.reason, "stamp");
    assert_eq!(music.unchanged, 2);
    assert_eq!(music.changed, 0);
    assert!(music.log_line().contains("skipped=true reason=stamp"));
}

#[test]
fn restored_mtime_keeps_the_old_content_hash() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (first, second) = music_tree(root.path());
    let stamped = FileTime::from_unix_time(1_700_000_000, 0);
    set_file_mtime(&first, stamped).unwrap();
    set_file_mtime(&second, stamped).unwrap();
    let (_, store) = open_at(data.path(), Some(root.path()), None);
    reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    let key = "Artist/Album/01-prelude.wav";
    let stored = store.content_hash(PluginId::Music, key).unwrap().unwrap();
    let mut bytes = fs::read(&first).unwrap();
    bytes[80] ^= 0xff;
    fs::write(&first, &bytes).unwrap();
    set_file_mtime(&first, stamped).unwrap();
    let report = reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    assert!(report.skipped, "{}", report.log_line());
    assert_eq!(
        store.content_hash(PluginId::Music, key).unwrap().unwrap(),
        stored
    );
    assert_ne!(sha(&first), stored);
}

#[test]
fn mtime_bump_with_the_same_bytes_does_not_rewrite_the_spine() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (first, _) = music_tree(root.path());
    let (_, store) = open_at(data.path(), Some(root.path()), None);
    reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    let id = store.items(Some(PluginId::Music), &[]).unwrap()[0]
        .id
        .clone();
    let spine = store.spine_path(&id);
    let frozen = FileTime::from_unix_time(1_500_000_000, 0);
    set_file_mtime(&spine, frozen).unwrap();
    let before = store
        .content_hash(PluginId::Music, "Artist/Album/01-prelude.wav")
        .unwrap();
    set_file_mtime(&first, FileTime::from_unix_time(1_800_000_000, 0)).unwrap();
    let report = reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    assert!(!report.skipped);
    assert_eq!(report.changed, 0);
    assert_eq!(
        store
            .content_hash(PluginId::Music, "Artist/Album/01-prelude.wav")
            .unwrap(),
        before
    );
    let seen = FileTime::from_last_modification_time(&fs::metadata(&spine).unwrap());
    assert_eq!(seen, frozen);
}

#[test]
fn changed_bytes_update_the_hash() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (first, _) = music_tree(root.path());
    let (_, store) = open_at(data.path(), Some(root.path()), None);
    reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    let mut bytes = fs::read(&first).unwrap();
    bytes[80] ^= 0xff;
    fs::write(&first, &bytes).unwrap();
    set_file_mtime(&first, FileTime::from_unix_time(1_900_000_000, 0)).unwrap();
    let report = reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    assert_eq!(report.changed, 1);
    assert_eq!(
        store
            .content_hash(PluginId::Music, "Artist/Album/01-prelude.wav")
            .unwrap()
            .unwrap(),
        sha(&first)
    );
}

#[test]
fn empty_directory_clears_music_and_a_missing_root_does_not() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    music_tree(root.path());
    let (_, store) = open_at(data.path(), Some(root.path()), None);
    reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    fs::remove_dir_all(root.path().join("Artist")).unwrap();
    let cleared = reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    assert_eq!(cleared.removed, 2);
    assert!(store.items(Some(PluginId::Music), &[]).unwrap().is_empty());
    music_tree(root.path());
    reconcile(&store, &crate::music::Music, Some(root.path()), false).unwrap();
    let kept = reconcile(&store, &crate::music::Music, None, false).unwrap();
    assert!(kept.skipped);
    assert_eq!(kept.reason, "missing-root");
    assert_eq!(store.items(Some(PluginId::Music), &[]).unwrap().len(), 1);
}

#[test]
fn books_follow_calibre_rows_and_leave_the_library_untouched() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    calibre(root.path(), false);
    let db = root.path().join("metadata.db");
    let before = sha(&db);
    let (_, store) = open_at(data.path(), None, Some(root.path()));
    let report = reconcile(&store, &crate::books::Books, Some(root.path()), false).unwrap();
    assert_eq!(report.changed, 1);
    assert_eq!(before, sha(&db));
    assert!(!db.with_extension("db-wal").exists());
    assert!(!PathBuf::from(format!("{}-wal", db.display())).exists());
    assert!(!PathBuf::from(format!("{}-shm", db.display())).exists());
    let persuasion = store.item("bok_1").unwrap().unwrap();
    let ItemBody::Book(book) = persuasion.body else {
        panic!("book")
    };
    assert_eq!(persuasion.title, "Persuasion");
    assert_eq!(
        book.authors,
        vec!["Jane Austen".to_string(), "A. N. Other".to_string()]
    );
    assert_eq!(
        book.tags,
        vec!["Classic".to_string(), "England".to_string()]
    );
    assert_eq!(book.publisher.as_deref(), Some("John Murray"));
    assert_eq!(book.series.as_deref(), Some("Novels"));
    assert_eq!(book.series_index, Some(1.0));
    assert_eq!(persuasion.year, Some(1817));
    assert_eq!(book.pages, None);
    assert_eq!(book.formats, vec!["epub".to_string()]);
    assert_eq!(book.description.as_deref(), Some("Anne & Wentworth"));
    assert_eq!(book.palette.bg, "#1f4fa8");
    assert!(book.cover_rel.is_none());
    let undated = store.item("bok_2").unwrap().unwrap();
    let ItemBody::Book(old) = undated.body else {
        panic!("book")
    };
    assert_eq!(undated.year, None);
    assert!(old.authors.is_empty());
    assert!(old.series_index.is_none());
    let escape = store.item("bok_3").unwrap().unwrap();
    let ItemBody::Book(escaped) = escape.body else {
        panic!("book")
    };
    assert!(escaped.cover_rel.is_none());
    let found = store
        .items(Some(PluginId::Books), &["wentworth".into()])
        .unwrap();
    assert_eq!(found.len(), 1);
    let nested_root = tempfile::tempdir().unwrap();
    calibre(nested_root.path(), true);
    let nested_data = tempfile::tempdir().unwrap();
    let (_, nested) = open_at(nested_data.path(), None, Some(nested_root.path()));
    reconcile(
        &nested,
        &crate::books::Books,
        Some(nested_root.path()),
        false,
    )
    .unwrap();
    assert!(nested.item("bok_1").unwrap().is_some());
}

#[test]
fn a_read_only_metadata_db_still_lists() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    calibre(root.path(), false);
    let db = root.path().join("metadata.db");
    let mut perms = fs::metadata(&db).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&db, perms).unwrap();
    let (_, store) = open_at(data.path(), None, Some(root.path()));
    let report = reconcile(&store, &crate::books::Books, Some(root.path()), false).unwrap();
    assert_eq!(report.changed, 1);
    assert!(store.item("bok_1").unwrap().is_some());
    assert!(!PathBuf::from(format!("{}-wal", db.display())).exists());
}

#[test]
fn reopening_the_catalog_skips_and_keeps_the_spine() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    music_tree(root.path());
    let cfg = Config::for_roots(
        data.path().to_path_buf(),
        Some(root.path().to_path_buf()),
        None,
    );
    let id = {
        let store = Store::open(data.path()).unwrap();
        scan(&store, &cfg, false).unwrap();
        let id = store.items(Some(PluginId::Music), &[]).unwrap()[0]
            .id
            .clone();
        let frozen = FileTime::from_unix_time(1_600_000_000, 0);
        set_file_mtime(store.spine_path(&id), frozen).unwrap();
        id
    };
    let store = Store::open(data.path()).unwrap();
    let reports = scan(&store, &cfg, false).unwrap();
    let music = reports.iter().find(|r| r.plugin == "music").unwrap();
    assert!(music.skipped, "{}", music.log_line());
    let seen = FileTime::from_last_modification_time(&fs::metadata(store.spine_path(&id)).unwrap());
    assert_eq!(seen, FileTime::from_unix_time(1_600_000_000, 0));
}

#[tokio::test]
async fn http_lists_audio_and_now_playing() {
    let root = tempfile::tempdir().unwrap();
    let books = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    music_tree(root.path());
    calibre(books.path(), false);
    let (cfg, store) = open_at(data.path(), Some(root.path()), Some(books.path()));
    let reports = scan(&store, &cfg, false).unwrap();
    let app = router(AppState::new(store, cfg, reports));

    let listed = request(
        &app,
        Request::builder()
            .uri("/api/items?plugin=music")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    let items: serde_json::Value = serde_json::from_slice(&listed.body).unwrap();
    let album = &items[0];
    assert_eq!(album["title"], "Album");
    assert!(album.get("haystack").is_none());
    assert!(album["body"]["tracks"][0].get("relPath").is_none());
    let id = album["id"].as_str().unwrap();

    let audio = request(
        &app,
        Request::builder()
            .uri(format!("/api/items/{id}/audio/0"))
            .header(header::RANGE, "bytes=0-3")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(audio.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&audio.body[..4], b"RIFF");

    let page = request(
        &app,
        Request::builder().uri("/").body(Body::empty()).unwrap(),
    )
    .await;
    let html = String::from_utf8(page.body).unwrap();
    assert!(html.contains("id=\"audio\""));
    assert!(html.contains("className = \"case\""));
    assert!(html.contains("className = \"shelf\""));

    let missing = request(
        &app,
        Request::builder()
            .uri("/api/items/..%2Fsecret/cover")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let posted = request(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/now-playing")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(
                r#"{{"itemId":"{id}","title":"Album","subtitle":"Artist","trackTitle":"01-prelude","trackIndex":0,"positionMs":10,"paused":false}}"#
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(posted.status, StatusCode::OK);
    let now = request(
        &app,
        Request::builder()
            .uri("/api/now-playing")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let now: serde_json::Value = serde_json::from_slice(&now.body).unwrap();
    assert_eq!(now["trackTitle"], "01-prelude");

    let books_list = request(
        &app,
        Request::builder()
            .uri("/api/items?plugin=books&q=wentworth")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let books_json: serde_json::Value = serde_json::from_slice(&books_list.body).unwrap();
    assert_eq!(books_json[0]["title"], "Persuasion");
    assert!(books_json[0]["body"].get("coverRel").is_none());

    let mcp = request(
        &app,
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"now_playing","arguments":{}}}"#,
            ))
            .unwrap(),
    )
    .await;
    let mcp_body = String::from_utf8(mcp.body).unwrap();
    assert!(mcp_body.contains("01-prelude"), "{mcp_body}");

    let spec = request(
        &app,
        Request::builder()
            .uri("/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let spec = String::from_utf8(spec.body).unwrap();
    assert!(spec.contains("/api/items"));
}

#[test]
fn mcp_lists_tools_and_searches() {
    let books = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    calibre(books.path(), false);
    let (cfg, store) = open_at(data.path(), None, Some(books.path()));
    scan(&store, &cfg, false).unwrap();
    let Reply::Response(list) = mcp::reply(
        &store,
        None,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    ) else {
        panic!("list");
    };
    assert!(list.contains("search"));
    assert!(list.contains("now_playing"));
    let Reply::Response(found) = mcp::reply(
        &store,
        None,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"persuasion","plugin":"books"}}}"#,
    ) else {
        panic!("search");
    };
    assert!(found.contains("Persuasion"));
    assert!(!found.contains("haystack"));

    let raw = r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#;
    let frame = format!("Content-Length: {}\r\n\r\n{raw}", raw.len());
    let mut output = Vec::new();
    mcp::serve(&store, Cursor::new(frame.into_bytes()), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Content-Length:"));
    assert!(text.contains("browse"));
}

#[test]
fn openapi_matches_the_committed_document() {
    let generated = crate::http::openapi_json();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi/openapi.json");
    if std::env::var("SPINES_WRITE_OPENAPI").is_ok() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &generated).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(committed, generated);
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
}

async fn request(app: &axum::Router, req: Request<Body>) -> Answer {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Answer { status, body }
}
