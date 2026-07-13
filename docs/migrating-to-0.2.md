# Migrating to 0.2

Version 0.2 replaces the 0.1 CPU image renderer. It is intentionally incompatible and ships no runtime shim.

## Removed

- `AlliumRenderer`
- `AlliumWorkerClient`
- `ImageFormat`
- `render` and encoded JPEG/PNG output
- `renderLayerCropped`
- `renderAllLayers`
- caller-managed asset byte injection
- the Skia WASM artifact and CPU image-layer protocol
- direct main-thread WASM rendering

## Replacement

Create a `BrowserRenderer` for a WebGL2 canvas, register font bytes, load a masterdata session, and create a stateful `BrowserScene`.

| 0.1 responsibility | 0.2 replacement |
| --- | --- |
| Render a complete encoded image | Draw a semantic scene with `BrowserScene.draw()` |
| Render every layer as WebP | Inspect authored layers and commands with `scene.dump()` |
| Hide an encoded layer | `setLayerVisible` or revision-checked `setLayerMasks` |
| Re-render dynamic frames | `advance(tick)` plus compact GPU patches |
| Infer clickable pixels | Use interaction regions and control bindings from the dump |
| Collect and upload assets manually | Resource URL provider plus bounded runtime cache |
| Parse text or enumerate glyphs in application code | Supply font bytes; WASM owns TMP, demand, layout, and SDF |

The renderer returns state and geometry, not application behavior. Navigation, copying numeric text, hover feedback, editing, selection, and overlay DOM remain host-owned.
