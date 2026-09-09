"""Release gates must reject mismatched, incomplete, or modified packages."""
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "scripts/release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def wheel(self, root, platform, version=None):
        v = release.VERSION
        path = root / f"onenote_cli-{v}-py3-none-{platform}.whl"
        with zipfile.ZipFile(path, "w") as z:
            z.writestr(f"onenote_cli-{v}.dist-info/METADATA", f"Name: onenote-cli\nVersion: {version or v}\n")
            suffix = ".exe" if platform == "win_amd64" else ""
            z.writestr(f"onenote_cli-{v}.data/scripts/onenote{suffix}", b"fixture executable")
        return path

    def sources(self, root):
        v = release.VERSION
        for prefix, extension in [(f"onenote_cli-{v}", ".tar.gz"), (f"onenote-cli-{v}", ".crate")]:
            with tarfile.open(root / (prefix + extension), "w:gz") as z:
                content = f'[package]\nversion = "{v}"\n'.encode()
                info = tarfile.TarInfo(prefix + "/Cargo.toml")
                info.size = len(content)
                z.addfile(info, io.BytesIO(content))

    def test_complete_matrix_and_checksum_tampering(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            windows = self.wheel(root, "win_amd64")
            with self.assertRaisesRegex(ValueError, "exactly one"):
                release.verify(root, None)
            self.wheel(root, "manylinux_2_17_x86_64.manylinux2014_x86_64")
            self.sources(root)
            release.verify(root, "v" + release.VERSION)
            release.verify(root, "v" + release.VERSION)
            with zipfile.ZipFile(windows, "a") as z:
                z.writestr("modified.txt", "unexpected mutation")
            with self.assertRaisesRegex(ValueError, "checksums changed"):
                release.verify(root, None)

    def test_version_platform_and_tag_mismatches(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            with self.assertRaisesRegex(ValueError, "metadata mismatch"):
                release.inspect_wheel(self.wheel(root, "win_amd64", "99.0.0"))
            with self.assertRaisesRegex(ValueError, "platform"):
                release.inspect_wheel(self.wheel(root, "macosx_11_0_arm64"))
            with self.assertRaisesRegex(ValueError, "Tag must"):
                release.verify(root, "v99.0.0")


if __name__ == "__main__":
    unittest.main()
