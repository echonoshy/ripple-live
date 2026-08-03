import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { viteStaticCopy } from 'vite-plugin-static-copy'

// https://vite.dev/config/
export default defineConfig({
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
})
