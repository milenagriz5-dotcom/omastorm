"""Documentation drift checks catch retired commands, links, and anchors."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts/tooling'))
import docs


class DocsTests(unittest.TestCase):
    def test_missing_links_and_anchors(self):
        with tempfile.TemporaryDirectory() as scratch:
            root=Path(scratch);page=root/'README.md';target=root/'guide.md'
            target.write_text('# Setup\n\n# Setup\n')
            page.write_text('[good](guide.md#setup-1) [bad](guide.md#missing) [missing](no.md)\n')
            errors=docs.check_file(page,root)
            self.assertEqual(len(errors),2)
            self.assertIn('missing local anchor',errors[0])
            self.assertIn('missing local link',errors[1])
            target.write_text('```qml\n# Fake heading\n```\n')
            page.write_text('[bad](guide.md#fake-heading) [bad media](docs/media/missing.md)\n')
            self.assertEqual(len(docs.check_file(page,root)),2)

    def test_retired_commands_in_code_and_intentional_generated_external_links(self):
        with tempfile.TemporaryDirectory() as scratch:
            root=Path(scratch);page=root/'README.md'
            page.write_text('```sh\nmise engine-pin\n```\n[web](https://example.invalid) [capture](review/demo.png)\n')
            self.assertEqual(len(docs.check_file(page,root)),1)
            page.write_text('[good](target/generated.png) [outside](../outside.md)\n')
            self.assertIn('escapes repository',docs.check_file(page,root)[0])

    def test_source_comments_must_name_existing_docs(self):
        with tempfile.TemporaryDirectory() as scratch:
            root=Path(scratch);(root/'docs').mkdir();(root/'docs/real.md').write_text('# Real\n')
            (root/'a.rs').write_text('//! See `docs/real.md`.\n// and docs/gone.md\n')
            (root/'b.md').write_text('docs/also-gone.md is prose, checked as a link elsewhere\n')
            errors=docs.check_sources(['a.rs','b.md','missing.rs'],root)
            self.assertEqual(errors,['a.rs:2: missing doc docs/gone.md'])


if __name__ == '__main__': unittest.main()
