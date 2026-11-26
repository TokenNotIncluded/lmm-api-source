"""Validate the LMM API 2.1 site profile before publishing its notice."""

from datetime import date, timedelta
from pathlib import Path
import argparse
import re
import tomllib


LANGUAGES = {"en", "zh-CN", "zh-TW"}
DOCUMENTS = {"user-agreement", "privacy-policy", "refund-policy"}
DATES = {"notice_date", "effective_date"}


def check(path, release):
    with path.open("rb") as source:
        config = tomllib.load(source)
    if config["version"] != 1 or config["default_language"] != "en":
        raise ValueError("expected legal configuration format 1 and English as the default")
    if set(config["documents"]) != DOCUMENTS or set(config["variables"]) != DATES:
        raise ValueError("unexpected legal documents or date variables")
    if release and config["published"] is not True:
        raise ValueError("the release configuration must set published = true")
    dates = config["variables"]
    if release or config["published"] or any(dates.values()):
        if not all(isinstance(v, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", v) for v in dates.values()):
            raise ValueError("both dates must use YYYY-MM-DD")
        if date.fromisoformat(dates["effective_date"]) - date.fromisoformat(dates["notice_date"]) < timedelta(days=14):
            raise ValueError("this update requires at least 14 calendar days' advance notice")

    root = Path(config.get("template_root", path.parent))
    texts = {}
    for document, localizations in config["documents"].items():
        if set(localizations) != LANGUAGES:
            raise ValueError(f"{document} requires English and both Chinese editions")
        for language, filename in localizations.items():
            if filename != f"{language}/{document}.md":
                raise ValueError(f"unexpected source for {document} in {language}")
            text = (root / filename).read_text(encoding="utf-8")
            metadata = text.splitlines()[2]
            if "2.1" not in metadata or sorted(re.findall(r"\{\{([^{}]+)\}\}", text)) != sorted(DATES):
                raise ValueError(f"{filename} has incorrect publication metadata")
            texts[language, document] = text
    for language in LANGUAGES:
        terms = texts[language, "user-agreement"]
        chapter = re.search(r"^## 7\.[^\n]*\n+(.*?)\n+## 8\.", terms, re.M | re.S)
        refund = texts[language, "refund-policy"]
        heading = re.search(r"^### 7\.1 ", refund, re.M)
        if not chapter or not heading or chapter[1].strip() != refund[heading.start():].strip():
            raise ValueError(f"{language} refund page differs from section 7 of the terms")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", nargs="?", type=Path, default=Path(__file__).with_name("site.toml"))
    parser.add_argument("--release", action="store_true", help="require publication and both dates")
    args = parser.parse_args()
    try:
        check(args.profile.resolve(), args.release)
    except (KeyError, ValueError, OSError, IndexError) as error:
        parser.exit(1, f"{error}\n")
    print("LMM API 2.1 profile verified.")
