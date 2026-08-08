"""The preflight that refuses to start a sweep nothing could score.

Every Terminal-Bench verifier installs pytest from PyPI inside the task
container before it can run a single test. Under a TLS-intercepting gateway
that install fails, the verifier never runs, and Harbor records reward 0 — a
result shaped exactly like a wrong answer. One 89-task sweep was lost that way.
"""

import importlib.util
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location("benchmark_tls_under_test", _SCRIPT)
benchmark = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(benchmark)


def issuer(monkeypatch, value):
    monkeypatch.setattr(benchmark, "pypi_tls_issuer", lambda *a, **k: value)


def test_a_public_ca_lets_the_run_proceed(monkeypatch):
    issuer(
        monkeypatch,
        "C=BE, O=GlobalSign nv-sa, CN=GlobalSign Atlas R3 DV TLS CA 2025 Q4",
    )
    benchmark.check_tls_not_intercepted()  # does not raise


def test_an_intercepting_gateway_stops_the_run_before_it_costs_anything(monkeypatch):
    """The exact issuer that turned a live 89-task sweep into 0/89."""
    issuer(monkeypatch, "C=US, O=Block, Inc., CN=Cloudflare Gateway CA")
    with pytest.raises(benchmark.InterceptedTLSError) as caught:
        benchmark.check_tls_not_intercepted()
    message = str(caught.value)
    # The operator has to know both what broke and what to do about it.
    assert "Cloudflare Gateway CA" in message
    assert "warp-cli disconnect" in message


@pytest.mark.parametrize(
    "name",
    ["Zscaler Root CA", "netskope certificate authority", "mitmproxy"],
)
def test_other_common_interceptors_are_caught_too(monkeypatch, name):
    issuer(monkeypatch, f"CN={name}")
    with pytest.raises(benchmark.InterceptedTLSError):
        benchmark.check_tls_not_intercepted()


def test_an_unreadable_issuer_does_not_block_the_run(monkeypatch, capsys):
    """Unknown is not intercepted.

    openssl may be missing, or the host may be briefly offline. A preflight
    that cannot see must not stand in the way of a run that would have worked;
    the real interception signal is unambiguous when it is there.
    """
    issuer(monkeypatch, None)
    benchmark.check_tls_not_intercepted()
    assert "skipping check" in capsys.readouterr().out


def test_the_issuer_is_read_through_the_egress_proxy(monkeypatch):
    """curl, not openssl, or the guard is blind on the hosts that need it most.

    A runner whose only route out is a CONNECT proxy resets a direct connection
    before any certificate is presented, so `openssl s_client` reports nothing and
    the check skips itself — on exactly the locked-down network where a corporate
    gateway is most likely to be intercepting. curl honours https_proxy.
    """
    calls = []

    class Completed:
        stdout = ""
        stderr = (
            "* Connected to proxy\n"
            "*  subject: CN=files.pythonhosted.org\n"
            "*  issuer: C=BE; O=GlobalSign nv-sa; CN=GlobalSign Atlas R3\n"
        )

    def fake_run(argv, **kwargs):
        calls.append(argv[0])
        return Completed()

    monkeypatch.setattr(benchmark.subprocess, "run", fake_run)
    assert benchmark.pypi_tls_issuer() == "C=BE; O=GlobalSign nv-sa; CN=GlobalSign Atlas R3"
    assert calls == ["curl"], "openssl must not be consulted when curl answers"


def test_openssl_still_answers_on_a_host_without_curl(monkeypatch):
    """Fallback, so removing curl degrades to the old behaviour rather than none."""

    class Completed:
        stdout = "issuer=C=US, O=Zscaler Inc., CN=Zscaler Root CA\n"
        stderr = ""

    def fake_run(argv, **kwargs):
        if argv[0] == "curl":
            raise OSError("no curl here")
        return Completed()

    monkeypatch.setattr(benchmark.subprocess, "run", fake_run)
    issuer_value = benchmark.pypi_tls_issuer()
    assert issuer_value is not None and "Zscaler" in issuer_value
