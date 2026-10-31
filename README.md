# Clone Bay

Copy and edit EVE Online settings files. One portable Windows exe, nothing to
install.

**[Download the latest release](https://github.com/kriller641/clone-bay/releases/latest)**

The exe isn't code signed, so Windows shows "Windows protected your PC" the
first time you run it. Choose "More info", then "Run anyway".

## What it does

- **Copies settings** from one character or account onto others: everything,
  or only the parts you choose. Each character keeps its own ship and module
  setup.
- **Edits individual settings**, such as orbit and warp-to distances, scanning,
  the autopilot, camera and sound.
- **Edits probe formations** and shares them with friends.
- **Puts overview and chat tabs in order.**
- **Restores a file from one of its backups.**

## Safety

- Close EVE first: the app won't change anything while EVE is running.
- Every file is backed up before it is changed, to `clone-bay-backups` in
  the settings folder. To undo, close EVE and choose "Restore from a backup".
- Anything the app can't read with certainty is left alone.

Character, ship and system names are looked up from CCP's public API; nothing
else leaves your PC.

## Check a download

Each release is built by GitHub from this repository, and its release notes
show how to check that your download matches.

## Build from source

Install [Rust](https://rustup.rs) and the Visual Studio C++ build tools, then:

    cargo build --release -p clone-bay

## Credits

Full licence texts are in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt),
which comes with every release.

- **CCP hf.**: ship names from the
  [Static Data Export](https://developers.eveonline.com/static-data) and
  character names from [ESI](https://esi.evetech.net), under CCP's
  [Developer License Agreement](https://developers.eveonline.com/license-agreement).
- **[carbonengine/blue](https://github.com/carbonengine/blue)** by CCP Games
  (MIT): the settings file format.
- **[blue-marshal-rs](https://github.com/TrueBrain/blue-marshal-rs)** by
  TrueBrain (MIT): the file format code follows it.
- **[EVE Wrench](https://github.com/eve-wrench/eve-wrench-app)** by Tim Kunze
  (MIT, per its README): a guide to what the settings mean.
- **[EANM](https://github.com/FontaineRiant/EANM)** by Bertral (MIT): where
  this project started.
- **[egui](https://github.com/emilk/egui)** and the other libraries built
  into the app.

© 2014 CCP hf. All rights reserved. "EVE", "EVE Online", "CCP", and all
related logos and images are trademarks or registered trademarks of CCP hf.

Clone Bay is a third-party tool. It is not made, endorsed or supported by
CCP hf.

## Licence

MIT. See [LICENSE](LICENSE).
