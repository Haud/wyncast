"""Tests for build.py — credential loading, .env parsing, and dist assembly."""

import json
import os
import shutil
import sys
import unittest
from pathlib import Path
from unittest import mock
from unittest.mock import patch

# Add the extension directory to the path so we can import build
EXTENSION_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(EXTENSION_DIR))

import build  # noqa: E402
from build import ENV_FILE, CREDENTIALS_FILE, load_credentials, parse_env_file


# ---------------------------------------------------------------------------
# parse_env_file
# ---------------------------------------------------------------------------

class TestParseEnvFile(unittest.TestCase):
    def test_basic_key_value(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text("FOO=bar\nBAZ=qux\n")
            self.assertEqual(parse_env_file(f), {"FOO": "bar", "BAZ": "qux"})

    def test_strips_single_quotes(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text("KEY='value'\n")
            self.assertEqual(parse_env_file(f), {"KEY": "value"})

    def test_strips_double_quotes(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text('KEY="value"\n')
            self.assertEqual(parse_env_file(f), {"KEY": "value"})

    def test_ignores_comments_and_blanks(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text("# comment\n\nKEY=val\n  # indented comment\n")
            self.assertEqual(parse_env_file(f), {"KEY": "val"})

    def test_ignores_lines_without_equals(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text("NO_EQUALS\nGOOD=yes\n")
            self.assertEqual(parse_env_file(f), {"GOOD": "yes"})

    def test_value_with_equals(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text("KEY=val=ue\n")
            self.assertEqual(parse_env_file(f), {"KEY": "val=ue"})

    def test_whitespace_around_key_and_value(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / ".env"
            f.write_text("  KEY  =  value  \n")
            self.assertEqual(parse_env_file(f), {"KEY": "value"})


# ---------------------------------------------------------------------------
# load_credentials
# ---------------------------------------------------------------------------

class TestLoadCredentials(unittest.TestCase):
    def test_env_vars_take_priority(self):
        with mock.patch.dict(os.environ, {"AMO_JWT_ISSUER": "env-issuer", "AMO_JWT_SECRET": "env-secret"}):
            self.assertEqual(load_credentials(), ("env-issuer", "env-secret"))

    def test_dot_env_file(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            env_file = Path(tmp) / ".env"
            env_file.write_text("AMO_JWT_ISSUER=file-issuer\nAMO_JWT_SECRET=file-secret\n")
            with mock.patch.dict(os.environ, {}, clear=True), \
                 mock.patch("build.ENV_FILE", env_file), \
                 mock.patch("build.CREDENTIALS_FILE", Path(tmp) / "nonexistent"):
                self.assertEqual(load_credentials(), ("file-issuer", "file-secret"))

    def test_amo_credentials_file(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            creds = Path(tmp) / ".amo-credentials"
            creds.write_text("legacy-issuer\nlegacy-secret\n")
            with mock.patch.dict(os.environ, {}, clear=True), \
                 mock.patch("build.ENV_FILE", Path(tmp) / "nonexistent"), \
                 mock.patch("build.CREDENTIALS_FILE", creds):
                self.assertEqual(load_credentials(), ("legacy-issuer", "legacy-secret"))

    def test_exits_when_no_credentials(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.dict(os.environ, {}, clear=True), \
                 mock.patch("build.ENV_FILE", Path(tmp) / "nonexistent"), \
                 mock.patch("build.CREDENTIALS_FILE", Path(tmp) / "also-nonexistent"):
                with self.assertRaises(SystemExit):
                    load_credentials()

    def test_env_vars_override_dot_env(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            env_file = Path(tmp) / ".env"
            env_file.write_text("AMO_JWT_ISSUER=file-issuer\nAMO_JWT_SECRET=file-secret\n")
            with mock.patch.dict(os.environ, {"AMO_JWT_ISSUER": "env-issuer", "AMO_JWT_SECRET": "env-secret"}), \
                 mock.patch("build.ENV_FILE", env_file):
                self.assertEqual(load_credentials(), ("env-issuer", "env-secret"))

    def test_dot_env_with_quotes(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            env_file = Path(tmp) / ".env"
            env_file.write_text("AMO_JWT_ISSUER='quoted-issuer'\nAMO_JWT_SECRET=\"quoted-secret\"\n")
            with mock.patch.dict(os.environ, {}, clear=True), \
                 mock.patch("build.ENV_FILE", env_file), \
                 mock.patch("build.CREDENTIALS_FILE", Path(tmp) / "nonexistent"):
                self.assertEqual(load_credentials(), ("quoted-issuer", "quoted-secret"))


# ---------------------------------------------------------------------------
# Dist assembly
# ---------------------------------------------------------------------------

class BuildTestCase(unittest.TestCase):
    """Base test case that cleans the dist directory before/after each test."""

    def setUp(self):
        if build.DIST_DIR.exists():
            shutil.rmtree(build.DIST_DIR)

    def tearDown(self):
        if build.DIST_DIR.exists():
            shutil.rmtree(build.DIST_DIR)


class TestFirefoxExtensionRoot(BuildTestCase):
    """Tests that Firefox extension files exist at the extension root."""

    def test_manifest_exists_at_root(self):
        self.assertTrue((EXTENSION_DIR / "manifest.json").exists())

    def test_background_js_exists_at_root(self):
        self.assertTrue((EXTENSION_DIR / "background.js").exists())

    def test_shared_files_exist_at_root(self):
        self.assertTrue((EXTENSION_DIR / "browser-polyfill.js").exists())
        self.assertTrue((EXTENSION_DIR / "background-core.js").exists())
        self.assertTrue((EXTENSION_DIR / "content_scripts" / "espn.js").exists())

    def test_manifest_is_mv2(self):
        manifest = json.loads((EXTENSION_DIR / "manifest.json").read_text())
        self.assertEqual(manifest["manifest_version"], 2)
        self.assertIn("browser_specific_settings", manifest)

    def test_no_chrome_files_at_root(self):
        self.assertFalse((EXTENSION_DIR / "offscreen.html").exists())
        self.assertFalse((EXTENSION_DIR / "offscreen.js").exists())


class TestAssembleChromeDist(BuildTestCase):
    """Tests for assembling the Chrome dist directory."""

    def test_creates_dist_directory(self):
        dist = build.assemble_chrome_dist()
        self.assertTrue(dist.exists())
        self.assertEqual(dist, build.DIST_DIR / "chrome")

    def test_copies_shared_files(self):
        dist = build.assemble_chrome_dist()
        self.assertTrue((dist / "browser-polyfill.js").exists())
        self.assertTrue((dist / "background-core.js").exists())
        self.assertTrue((dist / "content_scripts" / "espn.js").exists())

    def test_copies_chrome_specific_files(self):
        dist = build.assemble_chrome_dist()
        self.assertTrue((dist / "manifest.json").exists())
        self.assertTrue((dist / "background.js").exists())
        self.assertTrue((dist / "offscreen.html").exists())
        self.assertTrue((dist / "offscreen.js").exists())

    def test_does_not_include_firefox_files(self):
        dist = build.assemble_chrome_dist()
        manifest = json.loads((dist / "manifest.json").read_text())
        self.assertNotIn("browser_specific_settings", manifest)

    def test_manifest_is_mv3(self):
        dist = build.assemble_chrome_dist()
        manifest = json.loads((dist / "manifest.json").read_text())
        self.assertEqual(manifest["manifest_version"], 3)
        self.assertIn("offscreen", manifest["permissions"])
        self.assertIn("service_worker", manifest["background"])

    def test_offscreen_html_references_scripts(self):
        dist = build.assemble_chrome_dist()
        html = (dist / "offscreen.html").read_text()
        self.assertIn("browser-polyfill.js", html)
        self.assertIn("background-core.js", html)
        self.assertIn("offscreen.js", html)


class TestBothBrowsers(BuildTestCase):
    """Tests that apply to both browser targets."""

    def test_chrome_dist_shares_polyfill_with_root(self):
        cr_dist = build.assemble_chrome_dist()
        self.assertEqual(
            (EXTENSION_DIR / "browser-polyfill.js").read_text(),
            (cr_dist / "browser-polyfill.js").read_text(),
        )

    def test_chrome_dist_shares_core_with_root(self):
        cr_dist = build.assemble_chrome_dist()
        self.assertEqual(
            (EXTENSION_DIR / "background-core.js").read_text(),
            (cr_dist / "background-core.js").read_text(),
        )

    def test_chrome_dist_shares_content_script_with_root(self):
        cr_dist = build.assemble_chrome_dist()
        self.assertEqual(
            (EXTENSION_DIR / "content_scripts" / "espn.js").read_text(),
            (cr_dist / "content_scripts" / "espn.js").read_text(),
        )

    def test_both_targets_have_same_version(self):
        cr_dist = build.assemble_chrome_dist()
        ff_manifest = json.loads((EXTENSION_DIR / "manifest.json").read_text())
        cr_manifest = json.loads((cr_dist / "manifest.json").read_text())
        self.assertEqual(ff_manifest["version"], cr_manifest["version"])


class TestMainCli(BuildTestCase):
    """Tests for the CLI argument handling."""

    def test_no_args_exits(self):
        with patch.object(sys, "argv", ["build.py"]):
            with self.assertRaises(SystemExit) as ctx:
                build.main()
            self.assertEqual(ctx.exception.code, 1)

    def test_invalid_target_exits(self):
        with patch.object(sys, "argv", ["build.py", "opera"]):
            with self.assertRaises(SystemExit) as ctx:
                build.main()
            self.assertEqual(ctx.exception.code, 1)


if __name__ == "__main__":
    unittest.main()
