import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	kit: {
		// A single-page app inside the Tauri window.
		adapter: adapter({ fallback: 'index.html' })
	}
};

export default config;
