"""Tests for sync_env.py — 1Password credential syncing."""

import subprocess
from pathlib import Path
from unittest import mock

import pytest

from sync_env import check_op, read_field, main, ENV_FILE, OP_VAULT, OP_ITEM


class TestCheckOp:
    def test_returns_path_when_found(self):
        with mock.patch("shutil.which", return_value="/usr/local/bin/op"):
            assert check_op() == "/usr/local/bin/op"

    def test_exits_when_not_found(self):
        with mock.patch("shutil.which", return_value=None):
            with pytest.raises(SystemExit):
                check_op()


class TestReadField:
    def test_returns_stripped_value(self):
        result = subprocess.CompletedProcess(args=[], returncode=0, stdout="my-value\n", stderr="")
        with mock.patch("subprocess.run", return_value=result) as mock_run:
            assert read_field("/usr/local/bin/op", "username") == "my-value"
            mock_run.assert_called_once_with(
                ["/usr/local/bin/op", "read", f"op://{OP_VAULT}/{OP_ITEM}/username"],
                capture_output=True,
                text=True,
            )

    def test_exits_on_failure(self):
        result = subprocess.CompletedProcess(args=[], returncode=1, stdout="", stderr="not signed in")
        with mock.patch("subprocess.run", return_value=result):
            with pytest.raises(SystemExit):
                read_field("/usr/local/bin/op", "username")


class TestMain:
    def test_writes_env_file(self, tmp_path):
        env_file = tmp_path / ".env"

        def fake_read_field(op, field):
            return {"username": "my-issuer", "credential": "my-secret"}[field]

        with mock.patch("sync_env.check_op", return_value="/usr/local/bin/op"), \
             mock.patch("sync_env.read_field", side_effect=fake_read_field), \
             mock.patch("sync_env.ENV_FILE", env_file):
            main()

        content = env_file.read_text()
        assert "AMO_JWT_ISSUER=my-issuer\n" in content
        assert "AMO_JWT_SECRET=my-secret\n" in content
