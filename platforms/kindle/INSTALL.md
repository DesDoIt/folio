# Installing Folio OS on a Kindle

This guide takes you from a stock Kindle to Folio OS (the e-ink device build of the Folio reading
app), step by step. Allow about an hour the first time: most of it is the jailbreak, which you only
do once.

> ⚠️ **Read this first.** Jailbreaking and running unofficial software is at your own risk. It has
> been done on thousands of Kindles without trouble, but a mistake can leave a Kindle needing a
> factory reset, and in rare cases worse. Folio OS never changes the Kindle's own system files
> unless you turn on the optional boot menu, and the stock Kindle software stays fully usable.

**Contents**

1. [What you need](#1-what-you-need)
2. [Check your Kindle model and firmware](#2-check-your-kindle-model-and-firmware)
3. [Jailbreak the Kindle](#3-jailbreak-the-kindle)
4. [After the jailbreak: keep it jailbroken](#4-after-the-jailbreak-keep-it-jailbroken)
5. [Get the Folio OS files](#5-get-the-folio-files)
6. [Copy Folio OS to the Kindle](#6-copy-folio-to-the-kindle)
7. [First start](#7-first-start)
8. [Optional extras](#8-optional-extras)
9. [Updating and uninstalling](#9-updating-and-uninstalling) (and [upgrading from Kindle Linux](#upgrading-from-kindle-linux))
10. [Troubleshooting](#10-troubleshooting)

---

## 1. What you need

- **A Kindle.** Folio OS is developed and tested on the **Kindle Basic 7th generation (2014,
  model "KT2")**. Other Kindles from the same era with a 600×800 touch screen (Paperwhite 2 and 3,
  Voyage) are similar but untested; newer Kindles with higher-resolution screens are not supported yet.
- **A USB cable that carries data**, not just power. If the Kindle doesn't show up as a drive on
  your computer, try another cable. This is the most common problem.
- **A computer**: macOS, Linux, or Windows.
- **About 100 MB free** on the Kindle (Folio OS itself is ~65 MB).
- **Wi-Fi** for the jailbreak and for Folio OS's online features.
- To build Folio OS yourself (step 5): **Docker Desktop** (or Podman) and **git**.

Before you start, **copy your books off the Kindle** (they're in the `documents` folder on the USB
drive). Books bought from Amazon can always be downloaded again, but personal documents can't.

## 2. Check your Kindle model and firmware

The jailbreak you can use depends on the Kindle's firmware version.

1. On the Kindle, open **Settings** (the ☰ or ⋮ menu → **Settings**).
2. Go to **Device Options → Device Info** (on some versions: **⋮ → Device Info**).
3. Note the **Firmware Version** (for example `5.12.2.2`) and the **Serial Number**.

The first characters of the serial number identify the model (the Kindle Basic 7th gen starts with
`B0C6`, `90C6` and similar). The community keeps a full list in the
[Kindle models table](https://kindlemodding.org/kindle-hacking/models-table.html).

**Don't update the firmware** from here on. Amazon updates usually close jailbreaks. If the Kindle is
connected to Wi-Fi it may update itself, so keep it in **Airplane Mode** until you've jailbroken it and
blocked updates (step 4).

## 3. Jailbreak the Kindle

A jailbreak lets the Kindle run software that Amazon didn't sign. You only need to do it once.

The methods change as Amazon patches them, so this guide doesn't copy them. The Kindle modding
community at **[kindlemodding.org](https://kindlemodding.org/jailbreaking/)** keeps the
up-to-date, step-by-step guides. Use them:

1. Open the **[Jailbreak Wizard](https://kindlemodding.org/jailbreak-wizard.html)**. Enter the
   first 8 characters of your serial number and your firmware version. It tells you which method
   fits your Kindle.
2. Follow that method's guide exactly. The current methods include:

   | Method | Guide | Notes from the guide |
   |---|---|---|
   | **WinterBreak** | [kindlemodding.org/jailbreaking/WinterBreak](https://kindlemodding.org/jailbreaking/WinterBreak/) | Uses the Kindle Store. Needs Wi-Fi and a Kindle registered to an Amazon account. Doesn't work on firmware 5.18.1 or later. |
   | **WinterBreak2** | [kindlemodding.org/jailbreaking/WinterBreak2](https://kindlemodding.org/jailbreaking/WinterBreak2/) | Runs from the Kindle's web browser; needs Wi-Fi. The guide suggests the original WinterBreak for firmware below 5.16.4. |
   | Others (SpringBreak, Sanctuary, AdBreak, Nosebleed, LanguageBreak…) | [kindlemodding.org/jailbreaking](https://kindlemodding.org/jailbreaking/) | Pick the one the wizard recommends. |

   Folio OS was developed on a Kindle Basic 7th gen jailbroken with **WinterBreak2**.

3. Typical things the guides ask you to do (follow your guide, not this list):
   - Fill the Kindle's storage first so it can't download an update during the jailbreak
     ([how](https://kindlemodding.org/jailbreaking/prevent-auto-update/)), and delete the filler
     files afterwards.
   - Turn on **Airplane Mode** and restart the Kindle.
   - Download the jailbreak archive **from the GitHub link on the guide page** (it's open source),
     extract it **on your computer**, then copy the files to the Kindle over USB.
   - On macOS and Linux, make sure hidden files (names starting with a dot) are copied too.
   - Eject the Kindle properly before unplugging it.

**Is the download safe?** Only download jailbreak files from the links on kindlemodding.org, which
point to the community's GitHub releases. Be careful with copies posted elsewhere.

When the jailbreak finishes, the guide tells you how to confirm it worked (usually a message on
screen, or a new item in your library).

## 4. After the jailbreak: keep it jailbroken

Two things keep the jailbreak working:

- **The hotfix** re-applies the jailbreak after an Amazon update and adds support for running
  scripts from the library, which Folio OS uses. WinterBreak, SpringBreak and Sanctuary install
  it for you; for other methods, see
  [Setting up the hotfix](https://kindlemodding.org/jailbreaking/Legacy/post-jailbreak/setting-up-a-hotfix/).
- **Block automatic updates.** WinterBreak, SpringBreak and Sanctuary do this for you. Otherwise,
  see [Disabling OTA updates](https://kindlemodding.org/jailbreaking/Legacy/post-jailbreak/disable-ota.html).

**KUAL** (a launcher for Kindle add-ons) is optional: Folio OS doesn't need it. Install it if you
want other community extensions
([KUAL and MRPI](https://kindlemodding.org/jailbreaking/Legacy/post-jailbreak/installing-kual-mrpi/)).

You can turn Airplane Mode off again once updates are blocked.

## 5. Get the Folio OS files

There are no ready-made downloads yet, so you build the files on your computer. Docker does all
the work in a container; you don't need to install Rust or Linux tools.

1. Install **[Docker Desktop](https://docs.docker.com/get-docker/)** and start it.
   - On **Windows**, also install **WSL** (Windows Subsystem for Linux) and run the commands
     below in a WSL terminal (Ubuntu).
2. Download the source code:

   ```sh
   git clone https://github.com/DesDoIt/kindle-linux.git
   cd kindle-linux
   ```

3. Build:

   ```sh
   platforms/kindle/build/build.sh
   ```

   The first build takes about 5–15 minutes (it downloads the Rust compiler and Alpine Linux
   packages). Later builds take a minute or two. When it finishes you have two files in `out/`:
   `kos` (the app) and `kos-rootfs.sqsh` (the compressed Linux system, about 60 MB).

   If it fails with a DNS or "unable to select packages" error, Docker couldn't reach the internet:
   restart Docker Desktop and run it again.

You can try the interface on your computer first, without a Kindle: `platforms/kindle/build/cargo.sh sim` writes
screenshots of every screen to `sim-out/`.

## 6. Copy Folio OS to the Kindle

1. Connect the Kindle to your computer with the USB cable. It appears as a drive called **Kindle**.
2. Copy the files:

   **macOS:**

   ```sh
   platforms/kindle/build/install.sh
   ```

   **Linux** (give the path where the Kindle is mounted):

   ```sh
   platforms/kindle/build/install.sh /media/$USER/Kindle
   ```

   The script copies everything and checks every file after copying.

   **Windows, or by hand:** create a folder called `kos` at the top of the Kindle drive and copy
   these files:

   | From the project | To the Kindle |
   |---|---|
   | `out/kos-rootfs.sqsh` | `kos/` |
   | `out/kos` | `kos/` |
   | `platforms/kindle/host/kos-launch.sh`, `platforms/kindle/host/kos-boot.sh`, `platforms/kindle/host/folio.conf` | `kos/` |
   | `platforms/kindle/host/Folio.sh`, `platforms/kindle/host/Folio-BootOn.sh`, `platforms/kindle/host/Folio-BootOff.sh` | `documents/` |

3. **Eject** the Kindle (Finder: ⏏ next to Kindle; Windows: "Safely remove"), then unplug it.
   Unplugging without ejecting can leave half-copied files.

## 7. First start

1. On the Kindle's home screen, open your **library**. You'll see three new items:
   - **Folio OS**: starts Folio OS.
   - **Folio OS: boot ON** / **boot OFF**: the optional boot menu (see below).
2. Tap **Folio OS**. The screen flashes a few times while the Kindle's own interface stops and
   Folio OS starts. This takes about 10 seconds.
3. A short **setup guide** opens: menu text size, time zone, Wi-Fi, sleep screen and weather. You can
   skip it and change everything later in **Settings**.

**Finding your way around**

- The top bar has **‹** (back) and **⌂** (home) on every screen, plus a **Wi-Fi** button and the
  battery level.
- In a book, tap the right side to turn the page and the left side to go back. **Tap the top of the
  page** for the toolbar (back, home, chapters, bookmarks, search, text size).
- Rows with more actions have a **⋯** button.
- Press the **power button** to sleep; press it again to wake.

**Going back to the normal Kindle:** Home → **Power** → **Exit to stock Kindle**. You need the stock
Kindle for USB drive mode and for books bought from Amazon (Folio OS can't open those, because
of their DRM).

## 8. Optional extras

- **Boot menu.** Tap **Folio OS: boot ON** in the stock library. After that, every time the
  Kindle starts you get a 5-second menu that starts Folio OS by default, or the stock Kindle if
  you choose it. **Folio OS: boot OFF** removes it. If Folio OS ever fails to start, the
  next boot goes to the stock Kindle automatically.
- **Adding books.** Copy books into the `documents` folder over USB: EPUB, MOBI/AZW3 (without DRM),
  FB2, RTF, DOCX, TXT, PDF, DjVu, or comics (CBZ/CBR). Folio OS reads all of them itself; you
  don't need KOReader. Or use **Send to Folio** (upload from your phone or computer over Wi-Fi), or download them with
  **Get Books**.
- **SSH.** Settings → set a root password → turn on SSH, then `ssh root@<kindle-ip>`.

## 9. Updating and uninstalling

**Update:** pull the latest code, rebuild, and copy again:

```sh
git pull
platforms/kindle/build/build.sh
platforms/kindle/build/install.sh
```

Your books, notes, highlights, settings, saved Wi-Fi networks and SSH password are all kept. Books,
notes and reading data live on the USB drive, and settings live on the Kindle's own storage in
`/var/local/folio`, outside the files an update replaces.

### Upgrading from Kindle Linux

Folio OS used to be called **Kindle Linux**. Update it like any other update (`platforms/kindle/build/install.sh`, or copy
the files from step 6 by hand). Nothing you have is lost:

- **Library items.** `platforms/kindle/build/install.sh` replaces **Kindle Linux**, **Kindle Linux: boot ON** and
  **boot OFF** with **Folio OS**, **Folio OS: boot ON** and **Folio OS: boot OFF**. If you copy by hand, delete
  the old `KindleLinux.sh`, `KindleLinux-BootOn.sh` and `KindleLinux-BootOff.sh` from `documents/`
  (and `kindle-linux.conf` from `kos/`) yourself.
- **Settings, Wi-Fi networks, SSH key and password.** The first time Folio OS starts, it moves them
  from `/var/local/kindle-linux` to `/var/local/folio`.
- **Boot menu.** If you had turned it on, the Kindle keeps showing it (now titled Folio OS) after the
  update. On that first boot, Folio OS swaps its boot hook (`/etc/upstart/kindle-linux.conf`) for the
  new one (`/etc/upstart/folio.conf`). If you want to be sure, tap **Folio OS: boot ON** once: it
  installs the new hook and removes the old one. **Folio OS: boot OFF** removes either.
- **Reading progress sync** now reports this Kindle as "Folio OS (Kindle)" to the sync server.

**Uninstall:**

1. If you turned the boot menu on, tap **Folio OS: boot OFF** in the stock library first.
2. Connect over USB and delete the `kos` folder and the three `Folio OS*.sh` files in
   `documents`.

Your books in `documents` are not touched. A few kilobytes of settings stay in
`/var/local/folio` on the Kindle's internal storage; they're harmless, but you can remove them
before uninstalling from Folio OS's **Terminal** with `rm -rf /kindle/var/local/folio`.
To remove the jailbreak itself, see the community's
[unjailbreaking guide](https://kindlemodding.org/jailbreaking/faq/unjailbreaking.html).

## 10. Troubleshooting

| Problem | What to do |
|---|---|
| The Kindle doesn't appear on the computer | Use a data cable (many cables are charge-only), try another USB port, and wake the Kindle. |
| "Folio OS" isn't in the library | Check that `Folio.sh` is in `documents/`, and that the jailbreak's hotfix is installed (step 4): it's what makes scripts show up in the library. |
| Tapping it does nothing, or shows an error | Check `kos/kos.log` on the USB drive. Make sure all the files from step 6 are in `kos/`, and copy again if any are missing. |
| "Couldn't mount image" | The copy of `kos-rootfs.sqsh` is incomplete: copy it again and eject properly. |
| Wi-Fi doesn't connect | Tap the **Wi-Fi** button in the top bar, scan, and pick your network. Folio OS reconnects by itself after sleep; if it doesn't, the log shows why. |
| The screen is stuck | Hold the power button for about 15 seconds to restart into the stock Kindle. |
| Folio OS starts by itself and I want it to stop | Connect over USB and create an empty file called `disable` inside `kos/` (no extension). Delete it to allow starting again. |
| Touches land in the wrong place | Create `kos/debug-touch`, reproduce it, and attach `kos/touch.log` to a bug report. |

Logs: `kos/kos.log` (current run) and `kos/kos.log.prev` (previous run). Please include them when you
[open an issue](https://github.com/DesDoIt/kindle-linux/issues).
