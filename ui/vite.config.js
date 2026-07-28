import { defineConfig } from 'vite'
import solidPlugin from 'vite-plugin-solid'
import suidPlugin from '@suid/vite-plugin'

export default defineConfig({
	plugins: [suidPlugin(), solidPlugin()],
	server: {
		port: 3000,
		proxy: {
			// `pnpm dev` → local Sarca (override with VITE_DEV_PROXY)
			'/api': {
				target: process.env.VITE_DEV_PROXY || 'http://127.0.0.1:8001',
				changeOrigin: true,
			},
		},
	},
	build: {
		target: 'esnext',
		// Do not use manualChunks for solid/@suid — it creates circular chunks
		// ("Cannot access '$' before initialization") and a blank white page.
		// Size is kept under the warning via slim icon maps + dynamic imports
		// for marked/mammoth in FileViewer.
	},
	test: {
		environment: 'jsdom',
		globals: true,
		setupFiles: './src/test/setup.js',
		include: ['src/**/*.test.{js,jsx}'],
	},
})
