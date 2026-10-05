# Vendor regression workspace

Run `python3 tools/vendor-regression/run.py` from the repository root.
The script copies the current vendor packages into an isolated workspace with
its own checked-in dependency lock. Both packages' complete library tests run.
See [the patch inventory](../../docs/VENDOR_REGRESSION.md).

The fixtures are unmodified files from longbridge/gpui-kit commit
`0c830f4d257e69fdd17200650533ab4ca9a40cc0`, the source of both 0.7.0
packages. Their directory paths match upstream. They fill omissions in the
published crates' test inputs. The upstream Apache-2.0 license is included.
They are test data, not product assets or user notes.
