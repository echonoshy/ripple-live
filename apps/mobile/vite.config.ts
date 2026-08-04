import { readFileSync } from 'node:fs'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { viteStaticCopy } from 'vite-plugin-static-copy'

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  const tauriConfig = JSON.parse(
    readFileSync(new URL('./src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
  ) as { version: string }
  const clientBuild = `${tauriConfig.version}${mode === 'development' ? '-dev' : ''}`

  return {
  define: {
    __RIPPLE_CLIENT_BUILD__: JSON.stringify(clientBuild),
  },
  // Tauri supplies TAURI_DEV_HOST when deploying to a physical device. Binding
  // to it makes the Vite server reachable by the Android WebView for live reload.
  server: {
    host: process.env.TAURI_DEV_HOST ?? '127.0.0.1',
  },
  plugins: [
    react(),
    viteStaticCopy({
      targets: [
        {
          src: 'node_modules/@ricky0123/vad-web/dist/vad.worklet.bundle.min.js',
          dest: 'vad',
          rename: { stripBase: true },
        },
        {
          src: 'node_modules/@ricky0123/vad-web/dist/silero_vad_v5.onnx',
          dest: 'vad',
          rename: { stripBase: true },
        },
        {
          src: 'node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.mjs',
          dest: 'vad/ort',
          rename: { stripBase: true },
        },
        {
          src: 'node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.wasm',
          dest: 'vad/ort',
          rename: { stripBase: true },
        },
      ],
    }),
  ],
  }
})
