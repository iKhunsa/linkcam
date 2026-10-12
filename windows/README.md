# LinkCam para Windows (Insta360 Link 2C)

Controlador ligero en Rust para la **Insta360 Link 2C** en Windows. Un solo `.exe`
(~14 MB) con ventana al estilo del *Insta360 Link Controller*, CLI y un motor de
**gestos propios** que dispara atajos de Windows.

Port de [`fmontes/linkcam`](https://github.com/fmontes/linkcam) (macOS). El código macOS
queda intacto; todo lo de Windows vive en esta carpeta `windows/`.

## Uso rápido

```powershell
cd windows
powershell -ExecutionPolicy Bypass -File package.ps1   # compila y arma dist\LinkCam
.\dist\LinkCam\linkcam.exe                             # abre la ventana
```

Requiere Rust (MSVC) para compilar. En la carpeta `dist\LinkCam` van el `.exe` y los
modelos de manos en `models\`.

### CLI

```text
linkcam                         abre la ventana
linkcam status [--json]         zoom y modo
linkcam zoom <1..4|1.5x|150>    zoom (recorte digital centrado)
linkcam mode <nombre>           normal | whiteboard | overhead | deskview | auto-framing
linkcam image <control> <v|auto>  brightness|contrast|saturation|sharpness|white-balance
linkcam formats                 resoluciones disponibles (4K30, 1080p30, 720p30 ...)
linkcam snap [f.png] [4K30]     foto a resolución completa
linkcam record <seg> [f.mp4]    graba H.264
linkcam gestures                motor de gestos en consola
linkcam autostart on|off|status iniciar con Windows
linkcam vcam install|uninstall|status|run <s>|test   cámara virtual
linkcam selftest                prueba de punta a punta de todo lo anterior
linkcam list                    dispositivos de captura
```

## Qué funciona (verificado en una Link 2C real)

| Función | Estado |
|---|---|
| Preview en vivo, selector de resolución 4K30 … 720p24 (modos nativos de la cámara) | ✅ |
| Zoom 1×–4× (recorte real en cámara) | ✅ |
| Modos DeskView / Overhead (cambian la imagen, restauran a normal) | ✅ |
| Whiteboard (`04 01`, confirmado contra el controlador oficial; requiere los marcadores de esquina) | ✅ comando verificado |
| Auto Framing (`07 10`, confirmado contra el controlador oficial; el encuadre necesita una persona) | ✅ comando verificado |
| Brillo / contraste / saturación / nitidez / balance de blancos | ✅ |
| Escenas guardadas (zoom + modo) | ✅ |
| Foto a resolución completa, grabación H.264/MP4 | ✅ |
| Gestos propios (mano → atajo / lanzar app / comando de cámara), editor en la GUI | ✅ validado con fotos reales de manos; **falta probarlo con tu mano en vivo** |
| Efecto de fondo (desenfoque con segmentación de personas), texto superpuesto | ✅ en preview, fotos, grabaciones y cámara virtual |
| Audio en grabaciones (micrófono de la cámara) + reducción de ruido (compuerta espectral) | ✅ pista AAC en el MP4; no reencamina el micrófono en vivo |
| Cámara virtual «LinkCam Virtual Camera» (Windows 11, Media Foundation) | ✅ fuente de medios validada leyéndola con un `SourceReader`; **el registro COM necesita aprobar el aviso de administrador una vez** (`linkcam vcam install`) |
| Pan / tilt digital | ❌ la firmware de la 2C ignora los controles UVC (ver `docs/xu-map-link2c.md`) |
| Interruptores de gestos del oficial (Auto Framing/Whiteboard/Zoom) | no cambian ningún selector XU: son de la app, no hay nada que replicar |
| Reencaminar el micrófono en vivo con reducción de ruido | ❌ requeriría un driver de audio virtual |

## Gestos propios

La cámara reconoce en firmware solo palma / V / L y no se puede ampliar. Aquí la detección
corre **en el PC**: se analiza el video (≈5 fps en reposo, ≈10 fps con mano) con los modelos
MediaPipe de palma y landmarks (ONNX, `tract`, sin DLLs). Poses: palma abierta, puño, pulgar
arriba, V, índice, pellizco, y swipes izquierda/derecha/arriba/abajo.

Configuración: engranaje ⚙ → *Gesture control*, o `%APPDATA%\LinkCam\config.toml`:

```toml
custom_gestures_enabled = true

[[bindings]]
gesture = "peace"
hold_ms = 600
cooldown_ms = 1500
enabled = true
[bindings.action]
type = "keys"
keys = ["win", "ctrl", "right"]
```

Acciones: `keys` (cualquier chord), `launch` (programa/URL), `camera` (`set_mode`,
`toggle_mode`, `zoom_by`, `recenter`). Los swipes se miden desde el punto de vista del usuario.

## Arquitectura

```
crates/
  linkcam-core    dominio puro: unidades, modos, protocolo XU, config, gestos (sin Windows)
  linkcam-win     DirectShow + IKsControl (XU), controles UVC, Media Foundation, SendInput
  linkcam-vision  detector de palma + landmarks, clasificador, despachador (hold/cooldown/swipe)
  linkcam-app     CLI + GUI egui + hilo de cámara
  linkcam-vcam    DLL COM con la fuente de medios de la cámara virtual (cdylib)
```

* La cámara se controla con `usbvideo` estándar: `IKsControl` sobre los 3 nodos de extensión.
  El mapa medido está en [`docs/xu-map-link2c.md`](docs/xu-map-link2c.md).
* `probe.exe` (solo lectura salvo `try-mode/poke/mode-sweep --write`) sirve para repetir
  los escaneos en otras cámaras.
* Los modos de la cámara **solo se aplican con el stream abierto**; la app lo abre sola.

## Tests y calidad

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Licencias

MIT (igual que linkcam). Los modelos ONNX son de MediaPipe vía OpenCV Model Zoo, Apache-2.0
(`models/MODELS-LICENSE.txt`). Proyecto no afiliado a Insta360; úsalo bajo tu responsabilidad.
