// Used by `cdev --auto watch page.html`: bundles a .ts/.tsx/.jsx entry for the
// browser, rewriting project files for auto-watch through the sidecar.
// usage: bun cdev-bun-build.js <entry> <root> <sidecar-origin>
const [entry, root, api] = process.argv.slice(2);
const loaders = { ts: 'ts', tsx: 'tsx', js: 'js', jsx: 'jsx', mts: 'ts', mjs: 'js' };
const result = await Bun.build({
  entrypoints: [entry],
  target: 'browser',
  sourcemap: 'inline',
  plugins: [{
    name: 'cdev-auto',
    setup(build) {
      build.onLoad({ filter: /\.(m?[jt]sx?)$/ }, async (args) => {
        const src = await Bun.file(args.path).text();
        const loader = loaders[args.path.split('.').pop()] || 'js';
        if (!args.path.startsWith(root + '/') || args.path.includes('/node_modules/')) return { contents: src, loader };
        try {
          const rel = args.path.slice(root.length + 1);
          const res = await fetch(`${api}/api/instrument?file=${encodeURIComponent(rel)}`, { method: 'POST', body: src });
          return { contents: res.ok ? await res.text() : src, loader };
        } catch {
          return { contents: src, loader };
        }
      });
    },
  }],
});
if (!result.success) {
  console.error(result.logs.map(String).join('\n'));
  process.exit(1);
}
process.stdout.write(await result.outputs[0].text());
