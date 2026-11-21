#!/usr/bin/env python3
"""Build a demo library from the public-domain files in /tmp/spines-sample.

Music is Commons Ogg recordings. Books are Project Gutenberg epubs.
Nothing here is personal. The output directory is gitignored.
"""

import base64
import json
import shutil
import sqlite3
import sys
import zipfile
from pathlib import Path

from mutagen.flac import Picture
from mutagen.oggvorbis import OggVorbis
from PIL import Image, ImageDraw, ImageFont

SOURCE = Path("/tmp/spines-sample")
OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "demo-library")

BOOKS = [
    ("pg11", "Alice's Adventures in Wonderland", "Lewis Carroll", "Carroll, Lewis", 1865, 160, "Children", ["Fantasy"]),
    ("pg46", "A Christmas Carol", "Charles Dickens", "Dickens, Charles", 1843, 104, None, ["Ghost"]),
    ("pg84", "Frankenstein", "Mary Shelley", "Shelley, Mary", 1818, 280, None, ["Gothic"]),
    ("pg98", "A Tale of Two Cities", "Charles Dickens", "Dickens, Charles", 1859, 380, None, ["Historical"]),
    ("pg174", "The Picture of Dorian Gray", "Oscar Wilde", "Wilde, Oscar", 1890, 250, None, ["Gothic"]),
    ("pg345", "Dracula", "Bram Stoker", "Stoker, Bram", 1897, 418, None, ["Gothic"]),
    ("pg768", "Wuthering Heights", "Emily Bronte", "Bronte, Emily", 1847, 416, None, ["Gothic"]),
    ("pg1342", "Pride and Prejudice", "Jane Austen", "Austen, Jane", 1813, 432, None, ["Romance"]),
    ("pg1661", "The Adventures of Sherlock Holmes", "Arthur Conan Doyle", "Doyle, Arthur Conan", 1892, 307, None, ["Mystery"]),
    ("pg2701", "Moby Dick", "Herman Melville", "Melville, Herman", 1851, 635, None, ["Adventure"]),
]

ALBUMS = [
    ("Johann Sebastian Bach", "Keyboard and cello", ("bach", "bwv")),
    ("Ludwig van Beethoven", "Sonatas", ("beethoven", "moonlight", "pathetique", "für elise", "fur elise")),
    ("Johannes Brahms", "Waltzes", ("brahms",)),
    ("Frederic Chopin", "Piano pieces", ("chopin",)),
    ("Claude Debussy", "Piano", ("debussy", "children")),
    ("Wolfgang Amadeus Mozart", "Piano and canon", ("mozart", "kv.", "turkey")),
    ("Erik Satie", "Gnossiennes", ("satie", "gnossienne")),
    ("Franz Schubert", "Impromptus", ("schubert",)),
]


def album_for(name: str):
    lowered = name.lower()
    for artist, album, keys in ALBUMS:
        if any(key in lowered for key in keys):
            return artist, album
    return None


def cover(path: Path, title: str, hue: int) -> None:
    image = Image.new("RGB", (600, 600), (18, 16, 14))
    draw = ImageDraw.Draw(image)
    color = (hue % 256, (hue * 3) % 256, (255 - hue) % 256)
    draw.rectangle((36, 36, 564, 564), fill=color)
    draw.rectangle((70, 70, 530, 530), outline=(246, 242, 232), width=4)
    font = ImageFont.load_default()
    draw.text((96, 270), title[:28], fill=(246, 242, 232), font=font)
    image.save(path, "JPEG", quality=85)


# Source filenames keep performer credits and leftover Vorbis comments.
# Lofty prefers "album artist" over albumartist, so a leftover key splits an album.
TITLES = (
    ("BWV.847", "Prelude in C minor, BWV 847"),
    ("D minor Fugue", "Fugue in D minor"),
    ("Bach-Siloti", "Prelude in B minor"),
    ("CELLO SUITE", "Cello Suite No. 1, Prelude"),
    ("waltz01", "Waltz No. 1"),
    ("waltz06", "Waltz No. 6"),
    ("Gnossienne no 1", "Gnossienne No. 1"),
    ("Gnossienne 2", "Gnossienne No. 2"),
    ("Gnossienne 4", "Gnossienne No. 4"),
    ("Gnossienne 6", "Gnossienne No. 6"),
    ("2nd Movement", "Moonlight Sonata, II"),
    ("3rd Movement", "Moonlight Sonata, III"),
    ("Adagio cantabile", "Pathetique, II. Adagio"),
    ("Rondo", "Pathetique, III. Rondo"),
    ("B-flat1", "Impromptu in B-flat major, I"),
    ("B-flat2", "Impromptu in B-flat major, II"),
    ("Black-keys", "Etude in G-flat major, Op. 10 No. 5"),
    ("Op.29", "Impromptu in A-flat major, Op. 29"),
    ("opus64", "Waltz in D-flat major, Op. 64 No. 1"),
    ("Canon for 2", "Canon in C minor, K. 230"),
    ("Turkey March", "Rondo alla turca"),
    ("KV.265", "Twelve variations on Ah vous dirai-je, Maman"),
    ("Children", "Children's Corner"),
    ("enfants", "Noel des enfants"),
)


def track_title(name: str) -> str:
    for needle, title in TITLES:
        if needle in name:
            return title
    return name.replace("_", " ")


def embed(path: Path, jpeg: bytes, artist: str, album: str, title: str, number: int) -> None:
    audio = OggVorbis(path)
    audio.clear()
    audio["artist"] = artist
    audio["albumartist"] = artist
    audio["album"] = album
    audio["title"] = title
    audio["tracknumber"] = str(number)
    picture = Picture()
    picture.type = 3
    picture.mime = "image/jpeg"
    picture.data = jpeg
    audio["metadata_block_picture"] = [base64.b64encode(picture.write()).decode("ascii")]
    audio.save()


def build_music() -> int:
    root = OUT / "music"
    if root.exists():
        shutil.rmtree(root)
    groups = {}
    for src in sorted((SOURCE / "audio").glob("*.ogg")):
        found = album_for(src.name)
        if found is None:
            continue
        groups.setdefault(found, []).append(src)
    count = 0
    for index, ((artist, album), files) in enumerate(sorted(groups.items())):
        folder = root / artist / album
        folder.mkdir(parents=True)
        art = folder / "cover.jpg"
        cover(art, album, 40 + index * 28)
        jpeg = art.read_bytes()
        for number, src in enumerate(files, start=1):
            dest = folder / f"{number:02d}{src.suffix}"
            shutil.copy(src, dest)
            embed(dest, jpeg, artist, album, track_title(src.name), number)
            count += 1
    return count


def epub_cover(epub: Path, dest: Path) -> bool:
    with zipfile.ZipFile(epub) as archive:
        names = [name for name in archive.namelist() if name.lower().endswith((".jpg", ".jpeg", ".png"))]
        preferred = [name for name in names if "cover" in name.lower()] or names
        if not preferred:
            return False
        data = archive.read(preferred[0])
    image = Image.open(__import__("io").BytesIO(data)).convert("RGB")
    image.thumbnail((800, 1200))
    image.save(dest, "JPEG", quality=85)
    return True


def build_books() -> int:
    root = OUT / "books"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    db = sqlite3.connect(root / "metadata.db")
    db.executescript(
        """
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
        CREATE TABLE books_pages_link (book INTEGER, pages INTEGER);
        """
    )
    tags = {}
    for book_id, (pg, title, author, author_sort, year, pages, series, book_tags) in enumerate(BOOKS, start=1):
        folder_name = f"{author}/{title} ({book_id})"
        folder = root / folder_name
        folder.mkdir(parents=True)
        epub = SOURCE / "epub" / f"{pg}.epub"
        shutil.copy(epub, folder / f"{title}.epub")
        has_cover = 1 if epub_cover(epub, folder / "cover.jpg") else 0
        db.execute(
            "INSERT INTO books VALUES (?, ?, ?, ?, ?, NULL, ?, ?)",
            (book_id, title, title, author_sort, f"{year}-01-01 00:00:00+00:00", folder_name, has_cover),
        )
        db.execute("INSERT INTO authors VALUES (?, ?)", (book_id, author))
        db.execute("INSERT INTO books_authors_link VALUES (?, ?, ?)", (book_id, book_id, book_id))
        db.execute("INSERT INTO data VALUES (?, ?, 'EPUB')", (book_id, book_id))
        db.execute("INSERT INTO books_pages_link VALUES (?, ?)", (book_id, pages))
        db.execute(
            "INSERT INTO comments VALUES (?, ?, ?)",
            (book_id, book_id, f"<p>{title} by {author}. Public domain, Project Gutenberg.</p>"),
        )
        if series:
            db.execute("INSERT INTO series VALUES (?, ?)", (book_id, series))
            db.execute("INSERT INTO books_series_link VALUES (?, ?, ?)", (book_id, book_id, book_id))
            db.execute("UPDATE books SET series_index = 1 WHERE id = ?", (book_id,))
        for name in book_tags:
            tag_id = tags.setdefault(name, len(tags) + 1)
            if tag_id == len(tags):
                db.execute("INSERT INTO tags VALUES (?, ?)", (tag_id, name))
            db.execute(
                "INSERT INTO books_tags_link VALUES (?, ?, ?)",
                (book_id * 10 + tag_id, book_id, tag_id),
            )
    db.commit()
    return len(BOOKS)


def main() -> None:
    tracks = build_music()
    books = build_books()
    print(json.dumps({"music_tracks": tracks, "books": books, "out": str(OUT)}))


if __name__ == "__main__":
    main()
