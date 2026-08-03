// File contents rendered through `innerHTML` (markdown, docx) are attacker
// controlled: anyone who can upload or share a file could otherwise run script
// on the app origin and read the tokens in localStorage. Everything that
// reaches an `innerHTML` sink must pass through here first.
//
// DOMPurify is loaded lazily so it stays out of the main bundle; the viewer
// already awaits its parser import on the same path.
export async function sanitizeHtml(html) {
	if (!html) return ''
	const DOMPurify = (await import('dompurify')).default
	return DOMPurify.sanitize(html, {
		USE_PROFILES: { html: true },
		// Block SVG/MathML entirely and forbid anything that can navigate or
		// fetch on its own.
		FORBID_TAGS: ['style', 'form', 'iframe', 'object', 'embed', 'base', 'link', 'meta'],
		FORBID_ATTR: ['style', 'formaction', 'target', 'ping'],
	})
}

export default sanitizeHtml
