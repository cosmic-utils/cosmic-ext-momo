# Translating Toot

Translations live in `i18n/<language>/cosmic_ext_toot.ftl` and use Fluent. English (`i18n/en`) is the fallback. Keep message IDs and variables unchanged when translating values.

Run `python3 scripts/check_resources.py` to find duplicate message IDs, missing English fallback strings, and coverage by language. Missing translations deliberately use English; avoid copying English into other locales merely to raise coverage.

Settings, discovery/profile controls, and recovery messages now use Fluent. Existing Bulgarian, Dutch, Polish, and Swedish resources are partial. A native speaker should review translated wording, longer labels, and screen-reader output before a release.
