"""Backup archive checks run without Docker, network access, or real credentials."""
import datetime
from contextlib import closing
import importlib.util
import io
from pathlib import Path
import sqlite3
import tarfile
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("archive", Path(__file__).resolve().parents[1] / "scripts/archive.py")
archive = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(archive)


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="woodshop-archive-test-")
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)

    def fixture(self):
        source = self.root / "source"
        (source / "photos/1").mkdir(parents=True)
        with closing(sqlite3.connect(source / "woodshop.db")) as database:
            database.execute("CREATE TABLE marker(value TEXT)")
            database.execute("INSERT INTO marker VALUES('snapshot')")
            database.commit()
        (source / "photos/1/reference.jpg").write_bytes(b"test-photo-content")
        target = self.root / "backup.tar.gz"
        archive.create(source, target)
        return source, target

    def test_round_trip_keeps_database_and_photo(self):
        _, target = self.fixture()
        restored = self.root / "restored"
        archive.validate(target, restored)
        with closing(sqlite3.connect(restored / "woodshop.db")) as database:
            self.assertEqual(database.execute("SELECT value FROM marker").fetchone()[0], "snapshot")
        self.assertEqual((restored / "photos/1/reference.jpg").read_bytes(), b"test-photo-content")

    def test_rejects_links_traversal_and_duplicate_members(self):
        for kind in ("symlink", "hardlink", "traversal", "duplicate"):
            with self.subTest(kind=kind):
                target = self.root / f"{kind}.tar.gz"
                with tarfile.open(target, "w:gz") as tar:
                    entry = tarfile.TarInfo("photos/link" if kind != "traversal" else "photos/../../outside")
                    if kind in ("symlink", "hardlink"):
                        entry.type = tarfile.SYMTYPE if kind == "symlink" else tarfile.LNKTYPE
                        entry.linkname = "/etc/passwd"
                    tar.addfile(entry)
                    if kind == "duplicate":
                        tar.addfile(entry)
                with self.assertRaises(ValueError):
                    archive.validate(target, self.root / f"extracted-{kind}")
                self.assertFalse((self.root / f"extracted-{kind}").exists())

    def test_photo_corruption_is_detected_before_extraction(self):
        _, target = self.fixture()
        corrupt = self.root / "corrupt.tar.gz"
        with tarfile.open(target) as source, tarfile.open(corrupt, "w:gz") as output:
            for member in source.getmembers():
                if member.name == "photos/1/reference.jpg":
                    payload = b"changed-photo"
                    member.size = len(payload)
                    output.addfile(member, io.BytesIO(payload))
                else:
                    output.addfile(member, source.extractfile(member) if member.isfile() else None)
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            archive.validate(corrupt, self.root / "bad-restore")
        self.assertFalse((self.root / "bad-restore").exists())

    def test_retention_keeps_seven_distinct_days_and_four_sunday_weeks(self):
        backups = self.root / "retention"
        backups.mkdir()
        today = datetime.date(2026, 10, 5)
        created = []
        for age in range(36):
            day = today - datetime.timedelta(days=age)
            for hour in (8, 20):
                path = backups / f"woodshop-{day:%Y%m%d}T{hour:02}0000Z.tar.gz"
                path.touch()
                created.append((day, hour, path))
        archive.prune(backups)
        remaining = {(day, hour) for day, hour, path in created if path.exists()}
        expected_daily = {(today - datetime.timedelta(days=age), 20) for age in range(7)}
        sunday = today - datetime.timedelta(days=(today.weekday() + 1) % 7)
        expected_weekly = {(sunday - datetime.timedelta(weeks=week), 20) for week in range(4)}
        self.assertEqual(remaining, expected_daily | expected_weekly)


if __name__ == "__main__":
    unittest.main()
