# Insta360 Link 2C — extension-unit map (Windows)

Measured on real hardware: `USB\VID_2E1A&PID_4C03`, firmware reported by the official
Link Controller as v3.2.6.8. Method: `probe` (this repo) over `IKsControl` on the
standard `usbvideo` driver; behaviour confirmed by photographing the stream.
Raw scans live in `docs/scans/`.

## Transport

* Driver: stock `usbvideo` — no libusb/WinUSB needed.
* Access: DirectShow capture filter → `IKsTopologyInfo` finds 3 vendor nodes
  (`KSNODETYPE_DEV_SPECIFIC`, ids 1–3) → `IKsControl::KsProperty` with `KSP_NODE`
  (`Set` = XU GUID, `Id` = selector, `NodeId` = node, flags `GET|SET|TOPOLOGY`).
* The driver returns the real payload length in *bytes returned*; BASICSUPPORT
  `MembersSize` is **not** reliable. Read with a 256-byte buffer.

| Node | GUID | Role (from linkcam / Linux controller, confirmed present) |
|------|------|------|
| 1 | `FAF1672D-B71B-4793-8C91-7B1C9B7F95F8` | primary: modes, features, device info |
| 2 | `E307E649-4618-A3FF-82FC-2D8B5F216773` | image/AI parameters, detection boxes (sel 0x18) |
| 3 | `A8BD5DF2-1A98-474E-8DD0-D92672D194FA` | legacy AI controls |

The 2C answers to the **same three GUIDs** as the Link / Link 2.

## Video mode — node 1, selector `0x02` (61 bytes on the 2C)

Byte 0 = mode id, byte 1 = flags. The rest of the payload (pan/tilt/zoom block at
`0x26..`, `0x32`) is **different from the Link 2** (`0x7fff 0x7fff` and `0x0064`
instead of `3610`/`-29491200`), so the control must be written
**read–modify–write**, changing only bytes 0 and 1.

| Mode | byte0 | byte1 written | Verified |
|------|-------|---------------|----------|
| Normal | `00` | `00` | yes — echoes `00 00` (the official app leaves `00 10` after Auto Framing off) |
| Whiteboard | `04` | `01` | **confirmed against the official controller** (clicking Whiteboard reads back `04 01`); needs the corner markers, so no visible change without them |
| Overhead | `05` | `00` | yes — image rotates/rectifies, echoes `05` |
| DeskView | `06` | `10` | yes — confirmed in both our writes and the official controller (`06 10`) |
| Auto Framing | `07` | `10` | **confirmed against the official controller** (clicking Auto Framing reads back `07 10`, off returns `00 10`). The framing itself needs a person in view |
| Link 2 AI tracking | `01` | – | **rejected** on the 2C (`ff`) |

### How the mapping was verified

The official Link Controller was driven with synthetic clicks while `probe scan`
captured every selector: with its preview streaming, Auto Framing → `07 10`,
Whiteboard → `04 01`, DeskView → `06 10`. Selector 2 is the only one that changes
(plus the detection-box selector, node 2 `0x18`, which is noise).

### Must be streaming

The firmware only applies modes while a video stream is open. With no stream every
write reads back `ff 00` (rejected). The app keeps a stream open (preview or a
keep-alive capture) before writing.

### Readback

After a successful switch byte 0 echoes the mode id. `ff` = rejected / not applicable.
Other selectors touched by a mode change: node 1 `0x0b`, `0x19`, `0x1c`; node 2
`0x18`, `0x1e` (status/geometry, not needed for control).

## Standard controls (DirectShow)

* **Zoom** (`IAMCameraControl`): 100–400 (1×–4×), works and is a real in-camera
  crop — verified (200 → 2× centre crop).
* **Pan / Tilt**: the driver advertises ±145° / −90…+100° (in **degrees**) and
  accepts writes, but the 2C does **not** move the crop (verified at 2× zoom with
  ±20°/10°). The `0x7fff 0x7fff` fields in selector 2 change the projection but
  are not a plain pan/tilt. Pan/tilt of the 2C's view is therefore not exposed as
  a UVC control; the official controller likely does it in software.
* **Image**: brightness / contrast / saturation / sharpness 0–100, white balance
  2000–10000 K. Exposure is not exposed through `IAMCameraControl`.

## Feature mask — node 1, selector `0x1b` (2 bytes)

Reads `c4 01` (0x01c4) on this unit. Bit 8 is the AI-tracking bit on Link 2; on the
2C its meaning is **unconfirmed**, so LinkCam never writes it.

## Gesture checkboxes (official Settings → Gesture Control)

Toggling *Auto Framing*, *Whiteboard* and *Zoom* gestures in the official app changes
**no XU selector** (full scans before/after are identical apart from noise). Those switches
are therefore app-side, so there is no firmware control to replicate.

## Not found / caveats

* Gesture events are not exposed by the camera; custom gestures are detected on the PC
  (`linkcam-vision`).
* Selector `0x1a` (Link 2 live pan/tilt) ignores writes on the 2C.
* Scans taken while the camera is idle only differ in the detection-box selector
  (node 2, `0x18`); see `idle-1.txt` vs `idle-2.txt`.
