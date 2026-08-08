import { mount } from 'svelte';
// Self-hosted rather than fetched from a CDN: the dashboard has to render on a
// LAN with no internet, and a webfont that never arrives is a page of fallbacks.
import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/500.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource/ibm-plex-mono/400.css';
import '@fontsource/ibm-plex-mono/500.css';
import '@fontsource/ibm-plex-mono/600.css';
import './shell.css';
import App from './App.svelte';

const app = mount(App, { target: document.getElementById('app')! });

export default app;
