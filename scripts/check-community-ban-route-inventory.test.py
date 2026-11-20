#!/usr/bin/env python3
"""Fixture self-tests: every route shape the scanner can't classify must fail."""

from __future__ import annotations

import importlib.util
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "check-community-ban-route-inventory.py"
SPEC = importlib.util.spec_from_file_location("route_inventory", SCRIPT)
scanner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(scanner)

API_MOD = """
pub mod admin;
pub mod gifs;
pub mod profiles;
"""
GIFS = """
pub(crate) const SEARCH_PATH: &str = "/gifs/search";
"""
PROFILES = """
pub(crate) const SEARCH_PATH: &str = "/profiles/search";
"""
ADMIN = """
pub fn router(state: State) -> Router {
    Router::new().route("/probe", get(probe))
}
"""


def build_router(extra: str = "") -> str:
    return textwrap.dedent(
        f"""
        pub fn build_router(state: State) -> Router {{
            Router::new()
                .route(api::gifs::SEARCH_PATH, post(api::gifs::search))
                .route(api::profiles::SEARCH_PATH, post(api::profiles::search))
                .nest("/api/admin/v1", api::admin::router(state.clone()))
                {extra}
        }}
        """
    )


class RouteScannerFixtures(unittest.TestCase):
    def scan(self, router: str, admin: str = ADMIN, extra_sources=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "crates/buzz-relay/src"
            (src / "api").mkdir(parents=True)
            (src / "router.rs").write_text(router)
            (src / "api/mod.rs").write_text(API_MOD)
            (src / "api/gifs.rs").write_text(GIFS)
            (src / "api/profiles.rs").write_text(PROFILES)
            (src / "api/admin.rs").write_text(admin)
            for relative, source in (extra_sources or {}).items():
                path = src / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)
            saved = scanner.ROOT, scanner.RELAY_SRC
            scanner.ROOT, scanner.RELAY_SRC = root, src
            try:
                return scanner.collect_registered_routes()
            finally:
                scanner.ROOT, scanner.RELAY_SRC = saved

    def assert_rejected(self, message: str, router: str, admin: str = ADMIN):
        with self.assertRaisesRegex(ValueError, message):
            self.scan(router, admin)

    def test_unclassified_method_chains_are_rejected(self):
        for expression in (
            "get(read).on(MethodFilter::PATCH, write)",
            "get(read).on_service(MethodFilter::PATCH, svc)",
            "get(read).post_service(svc)",
            "on(MethodFilter::PATCH, write)",
            "on_service(MethodFilter::PATCH, svc)",
            "get_service(svc)",
            "get(read).custom(write)",
            "wrap(get(read))",
            "opaque.get(read)",
        ):
            with self.subTest(expression=expression):
                self.assert_rejected(
                    "unclassified route method expression",
                    build_router(f'.route("/mixed", {expression})'),
                )

    def test_supported_method_chains_are_preserved(self):
        for expression in (
            "get(read).post(write).head(headers)",
            "axum::routing::get(read).post(write).head(headers)",
            "get(read).with_state(state.clone()).post(write).head(headers)",
            "get(read).layer(middleware(layer_arg())).post(write).route_layer(layer).head(headers)",
            "get(read) /* .on(MethodFilter::PATCH, write) */ .post(write).head(headers)",
        ):
            with self.subTest(expression=expression):
                routes = self.scan(build_router(f'.route("/mixed", {expression},)'))
                self.assertEqual(
                    {method for listener, method, path in routes if path == "/mixed"},
                    {"GET", "POST", "HEAD"},
                )

    def test_all_known_http_helpers_are_classified(self):
        for helper, method in scanner.HTTP_METHODS.items():
            with self.subTest(helper=helper):
                routes = self.scan(build_router(f'.route("/method", {helper}(handler))'))
                self.assertEqual(
                    {verb for listener, verb, path in routes if path == "/method"},
                    {method},
                )

    def test_calls_inside_handler_and_layer_arguments_are_not_methods(self):
        routes = self.scan(build_router(
            '.route("/args", get(|| async { post(payload); "patch(fake)" })'
            '.layer(make_layer(options(config))))'
        ))
        self.assertEqual(
            {method for listener, method, path in routes if path == "/args"},
            {"GET"},
        )

    def test_nested_builder_merge_is_rejected(self):
        self.assert_rejected(
            r"\.merge registration is not classified",
            build_router(),
            ADMIN.replace('get(probe))', 'get(probe)).merge(crate::api::profiles::router(state))'),
        )

    def test_unclassified_root_merge_is_rejected(self):
        self.assert_rejected(
            r"\.merge registration is not classified",
            build_router(".merge(api::admin::router(state.clone()))"),
        )

    def test_existing_root_merge_alias_is_allowed_once(self):
        self.scan(build_router(".merge(media_router)"))
        self.assert_rejected(
            r"allow-listed \.merge registration appears more than once",
            build_router(".merge(media_router).merge(media_router)"),
        )

    def test_git_settings_merge_requires_root_parent(self):
        sources = {
            "api/git/transport.rs": """
pub fn git_router(state: State) -> Router {
    Router::new()
        .route("/git/{owner}/{repo}/info/refs", get(info))
        .merge(super::settings::router())
}
""",
            "api/git/settings.rs": """
pub fn router() -> Router {
    Router::new().route("/git/{owner}/{repo}/default-branch", get(read).put(write))
}
""",
        }
        routes = self.scan(build_router(".merge(git_router)"), extra_sources=sources)
        self.assertIn(("relay", "GET", "/git/{owner}/{repo}/default-branch"), routes)
        self.assertIn(("relay", "PUT", "/git/{owner}/{repo}/default-branch"), routes)
        with self.assertRaisesRegex(ValueError, r"allow-listed \.merge in a nested builder"):
            self.scan(
                build_router('.nest("/alternate", api::git::transport::git_router(state))'),
                extra_sources=sources,
            )

    def test_root_merge_alias_moved_inside_nest_is_rejected(self):
        self.assert_rejected(
            r"root \.merge registration inside a \.nest target is not classified",
            build_router().replace(
                'api::admin::router(state.clone()))',
                'api::admin::router(state.clone()).merge(git_policy_router))',
            ),
        )

    def test_merge_outside_router_builder_is_rejected(self):
        self.assert_rejected(
            r"\.merge registration is not classified",
            build_router() + "\nfn hidden() { opaque.merge(private_router); }",
        )

    def test_same_named_constants_resolve_per_module(self):
        routes = self.scan(build_router())
        self.assertEqual(
            set(routes),
            {
                ("relay", "POST", "/gifs/search"),
                ("relay", "POST", "/profiles/search"),
                ("relay", "GET", "/api/admin/v1/probe"),
            },
        )

    def test_spa_fallback_service_is_allowed(self):
        self.scan(build_router(".fallback_service(spa_fallback)"))

    def test_route_service_is_rejected(self):
        self.assert_rejected(
            r"\.route_service registration is not classified",
            build_router('.route_service("/probe", service)'),
        )

    def test_nest_service_is_rejected(self):
        self.assert_rejected(
            r"\.nest_service registration is not classified",
            build_router('.nest_service("/probe", service)'),
        )

    def test_fallback_service_outside_build_router_is_rejected(self):
        self.assert_rejected(
            r"\.fallback_service registration is not classified",
            build_router(),
            ADMIN.replace('get(probe))', "get(probe)).fallback_service(service)"),
        )

    def test_non_spa_fallback_service_in_build_router_is_rejected(self):
        self.assert_rejected(
            r"\.fallback_service registration is not classified",
            build_router(
                ".fallback_service(spa_fallback)"
                '.merge(Router::new().route("/x", get(x)).fallback_service(private_service))'
            ),
        )

    def test_repeated_spa_fallback_service_is_rejected(self):
        self.assert_rejected(
            r"\.fallback_service registration appears more than once",
            build_router(".fallback_service(spa_fallback).fallback_service(spa_fallback)"),
        )

    def test_handler_fallback_is_rejected(self):
        self.assert_rejected(
            r"\.fallback registration is not classified",
            build_router(".fallback(private_handler)"),
        )

    def test_handler_fallback_outside_build_router_is_rejected(self):
        self.assert_rejected(
            r"\.fallback registration is not classified",
            build_router(),
            ADMIN.replace('get(probe))', "get(probe)).fallback(private_handler)"),
        )

    def test_builder_nested_twice_is_rejected(self):
        self.assert_rejected(
            r"api::admin::router is nested more than once",
            build_router('.nest("/alternate-admin", api::admin::router(state.clone()))'),
        )

    def test_nest_outside_build_router_is_rejected(self):
        self.assert_rejected(
            r"\.nest registration outside router::build_router",
            build_router(),
            ADMIN.replace('get(probe))', 'get(probe)).nest("/zz", other_router(state))'),
        )

    def test_unresolved_nest_target_is_rejected(self):
        self.assert_rejected(
            r"nest target missing::router matches 0 router builders",
            build_router('.nest("/zz", missing::router(state.clone()))'),
        )

    def test_unknown_route_constant_is_rejected(self):
        self.assert_rejected(
            r"not a literal or known string constant: api::gifs::MISSING",
            build_router(".route(api::gifs::MISSING, get(missing))"),
        )


if __name__ == "__main__":
    unittest.main()
