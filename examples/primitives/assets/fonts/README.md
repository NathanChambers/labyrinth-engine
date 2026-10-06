# Font assets

Place application-specific font files beside the application that owns them.

The checked-in default is Roboto Regular, licensed under the Apache License 2.0. See the upstream project at <https://github.com/googlefonts/roboto> for the complete license and notices.

The primitives application resolves this file from its package directory with `FontAsset::from_file`, retains the returned asset, and assigns a clone to `UiCanvas::set_font`. The engine does not select or load fonts implicitly, so the same asset reference can be shared with other systems.
