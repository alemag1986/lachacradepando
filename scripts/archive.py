#!/usr/bin/env python3
"""Portable, strict archive handling for Woodshop data backups."""
import datetime
import hashlib
import pathlib
import re
import sys
import tarfile


def files_under(root: pathlib.Path) -> list[pathlib.Path]:
    return sorted(path for path in root.rglob("*") if path.is_file() and not path.is_symlink())


def create(source: pathlib.Path, destination: pathlib.Path) -> None:
    files = [source / "woodshop.db", *files_under(source / "photos")]
    lines = []
    for path in files:
        relative = path.relative_to(source).as_posix()
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        lines.append(f"{digest}  {relative}\n")
    (source / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    with tarfile.open(destination, "w:gz") as archive:
        archive.add(source / "woodshop.db", arcname="woodshop.db", recursive=False)
        archive.add(source / "SHA256SUMS", arcname="SHA256SUMS", recursive=False)
        archive.add(source / "photos", arcname="photos", recursive=True, filter=regular_filter)
    validate(destination, None)


def regular_filter(info: tarfile.TarInfo) -> tarfile.TarInfo:
    if not (info.isfile() or info.isdir()):
        raise ValueError(f"refusing non-regular file: {info.name}")
    return info


def validate(archive_path: pathlib.Path, destination: pathlib.Path | None) -> None:
    with tarfile.open(archive_path, "r:gz") as archive:
        members = archive.getmembers()
        names = [member.name for member in members]
        if len(names) != len(set(names)):
            raise ValueError("duplicate archive paths")
        for member in members:
            path = pathlib.PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or "\\" in member.name:
                raise ValueError(f"unsafe archive path: {member.name}")
            if not (member.isfile() or member.isdir()):
                raise ValueError(f"links and special files are forbidden: {member.name}")
            if not (member.name in {"woodshop.db", "SHA256SUMS", "photos"} or member.name.startswith("photos/")):
                raise ValueError(f"unexpected archive path: {member.name}")

        manifest_file = archive.extractfile("SHA256SUMS")
        if manifest_file is None:
            raise ValueError("missing SHA256SUMS")
        manifest = manifest_file.read().decode("utf-8")
        expected_names = set()
        for line in manifest.splitlines():
            digest, name = line.split("  ", 1)
            member = archive.getmember(name)
            content = archive.extractfile(member)
            if content is None or hashlib.sha256(content.read()).hexdigest() != digest:
                raise ValueError(f"checksum mismatch: {name}")
            expected_names.add(name)
        actual_names = {member.name for member in members if member.isfile() and member.name != "SHA256SUMS"}
        if actual_names != expected_names:
            raise ValueError("manifest does not cover every regular data file")
        if "woodshop.db" not in expected_names or "photos" not in names:
            raise ValueError("backup is missing database or photos directory")

        if destination is not None:
            destination.mkdir(mode=0o700)
            for member in members:
                target = destination.joinpath(*pathlib.PurePosixPath(member.name).parts)
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    content = archive.extractfile(member)
                    if content is None:
                        raise ValueError(f"cannot read: {member.name}")
                    with target.open("xb") as output:
                        output.write(content.read())
                    target.chmod(0o600)


def prune(directory: pathlib.Path) -> None:
    archives = []
    for path in directory.iterdir():
        match = re.fullmatch(r"woodshop-(\d{8})T(\d{6})Z\.tar\.gz", path.name)
        if match and path.is_file():
            stamp = datetime.datetime.strptime("".join(match.groups()), "%Y%m%d%H%M%S").replace(tzinfo=datetime.timezone.utc)
            archives.append((stamp, path))
    archives.sort(reverse=True)
    daily_dates: set[datetime.date] = set()
    weekly_dates: set[datetime.date] = set()
    keep = set()
    for stamp, path in archives:
        day = stamp.date()
        week = day - datetime.timedelta(days=(day.weekday() + 1) % 7)
        if day not in daily_dates and len(daily_dates) < 7:
            daily_dates.add(day)
            keep.add(path)
        if stamp.weekday() == 6 and week not in weekly_dates and len(weekly_dates) < 4:
            weekly_dates.add(week)
            keep.add(path)
    for _, path in archives:
        if path not in keep:
            path.unlink()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("usage: archive.py create|extract|prune ...")
    operation = sys.argv[1]
    if operation == "create":
        create(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]))
    elif operation == "extract":
        validate(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]))
    elif operation == "prune":
        prune(pathlib.Path(sys.argv[2]))
    else:
        raise SystemExit("usage: archive.py create|extract|prune ...")
