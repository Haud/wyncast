"""Tests for build.py — credential loading and .env parsing."""

import os
import textwrap
from pathlib import Path
from unittest import mock

import pytest

from build import ENV_FILE, CREDENTIALS_FILE, load_credentials, parse_env_file


# ---------------------------------------------------------------------------
# parse_env_file
# ---------------------------------------------------------------------------

class TestParseEnvFile:
    def test_basic_key_value(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text("FOO=bar\nBAZ=qux\n")
        assert parse_env_file(f) == {"FOO": "bar", "BAZ": "qux"}

    def test_strips_single_quotes(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text("KEY='value'\n")
        assert parse_env_file(f) == {"KEY": "value"}

    def test_strips_double_quotes(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text('KEY="value"\n')
        assert parse_env_file(f) == {"KEY": "value"}

    def test_ignores_comments_and_blanks(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text("# comment\n\nKEY=val\n  # indented comment\n")
        assert parse_env_file(f) == {"KEY": "val"}

    def test_ignores_lines_without_equals(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text("NO_EQUALS\nGOOD=yes\n")
        assert parse_env_file(f) == {"GOOD": "yes"}

    def test_value_with_equals(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text("KEY=val=ue\n")
        assert parse_env_file(f) == {"KEY": "val=ue"}

    def test_whitespace_around_key_and_value(self, tmp_path):
        f = tmp_path / ".env"
        f.write_text("  KEY  =  value  \n")
        assert parse_env_file(f) == {"KEY": "value"}


# ---------------------------------------------------------------------------
# load_credentials
# ---------------------------------------------------------------------------

class TestLoadCredentials:
    def test_env_vars_take_priority(self, tmp_path, monkeypatch):
        """Environment variables should win over all file-based methods."""
        monkeypatch.setenv("AMO_JWT_ISSUER", "env-issuer")
        monkeypatch.setenv("AMO_JWT_SECRET", "env-secret")
        assert load_credentials() == ("env-issuer", "env-secret")

    def test_dot_env_file(self, tmp_path, monkeypatch):
        """Falls back to .env file when env vars are absent."""
        monkeypatch.delenv("AMO_JWT_ISSUER", raising=False)
        monkeypatch.delenv("AMO_JWT_SECRET", raising=False)

        env_file = tmp_path / ".env"
        env_file.write_text("AMO_JWT_ISSUER=file-issuer\nAMO_JWT_SECRET=file-secret\n")

        with mock.patch("build.ENV_FILE", env_file), \
             mock.patch("build.CREDENTIALS_FILE", tmp_path / "nonexistent"):
            assert load_credentials() == ("file-issuer", "file-secret")

    def test_amo_credentials_file(self, tmp_path, monkeypatch):
        """Falls back to .amo-credentials when env vars and .env are absent."""
        monkeypatch.delenv("AMO_JWT_ISSUER", raising=False)
        monkeypatch.delenv("AMO_JWT_SECRET", raising=False)

        creds = tmp_path / ".amo-credentials"
        creds.write_text("legacy-issuer\nlegacy-secret\n")

        with mock.patch("build.ENV_FILE", tmp_path / "nonexistent"), \
             mock.patch("build.CREDENTIALS_FILE", creds):
            assert load_credentials() == ("legacy-issuer", "legacy-secret")

    def test_exits_when_no_credentials(self, tmp_path, monkeypatch):
        """Should sys.exit when no credentials source is available."""
        monkeypatch.delenv("AMO_JWT_ISSUER", raising=False)
        monkeypatch.delenv("AMO_JWT_SECRET", raising=False)

        with mock.patch("build.ENV_FILE", tmp_path / "nonexistent"), \
             mock.patch("build.CREDENTIALS_FILE", tmp_path / "also-nonexistent"):
            with pytest.raises(SystemExit):
                load_credentials()

    def test_env_vars_override_dot_env(self, tmp_path, monkeypatch):
        """Env vars should take priority even when .env file exists."""
        monkeypatch.setenv("AMO_JWT_ISSUER", "env-issuer")
        monkeypatch.setenv("AMO_JWT_SECRET", "env-secret")

        env_file = tmp_path / ".env"
        env_file.write_text("AMO_JWT_ISSUER=file-issuer\nAMO_JWT_SECRET=file-secret\n")

        with mock.patch("build.ENV_FILE", env_file):
            assert load_credentials() == ("env-issuer", "env-secret")

    def test_dot_env_with_quotes(self, tmp_path, monkeypatch):
        """.env values with quotes should be stripped."""
        monkeypatch.delenv("AMO_JWT_ISSUER", raising=False)
        monkeypatch.delenv("AMO_JWT_SECRET", raising=False)

        env_file = tmp_path / ".env"
        env_file.write_text("AMO_JWT_ISSUER='quoted-issuer'\nAMO_JWT_SECRET=\"quoted-secret\"\n")

        with mock.patch("build.ENV_FILE", env_file), \
             mock.patch("build.CREDENTIALS_FILE", tmp_path / "nonexistent"):
            assert load_credentials() == ("quoted-issuer", "quoted-secret")
