# Third-party software

Workboard is MIT-licensed. The UI bundles these libraries so it works offline (nothing is loaded from a CDN).
npm packages are pinned in `package.json` and their licenses ship inside `node_modules/<package>/LICENSE`.

| Component | Version | License | Where | Used for |
|---|---|---|---|---|
| [xterm.js](https://github.com/xtermjs/xterm.js) (`@xterm/xterm`, `@xterm/addon-fit`) | 5.5.0 / 0.10.0 | MIT | npm, bundled by Vite | Agent session terminals |
| [Mermaid](https://github.com/mermaid-js/mermaid) | 11.17.2 | MIT | npm, bundled by Vite as on-demand chunks | Mermaid files and ` ```mermaid ` blocks in the file viewer |
| [draw.io viewer](https://github.com/jgraph/drawio) (`viewer-static.min.js`, `shapes/`, `styles/`, `mxgraph/`) | 24.7.17 | Apache 2.0 (JGraph adapted), see `ui/public/vendor/drawio/LICENSE` | `ui/public/vendor/drawio/`, loaded on demand | draw.io diagrams in the file viewer |
| draw.io stencils and icon images (`stencils/`, `img/lib/`) | 24.7.17 | CC BY 4.0 (stencils, see `stencils/LICENSE`); vendor icon sets keep their owners' terms | `ui/public/vendor/drawio/` | Shape libraries for draw.io diagrams |

## draw.io subset

The full draw.io viewer with every shape library is about 53 MB. Workboard ships a subset of about 16 MB
(uncompressed; Tauri compresses embedded assets):

- `viewer-static.min.js` (3.5 MB), `shapes/` (2.3 MB), `styles/`, `mxgraph/images/`.
- `stencils/`: the general libraries up to Azure in size (basic, arrows, flowchart, BPMN, ER-like, networks,
  Kubernetes, Android/iOS mockups, Atlassian, IBM, sitemap, floorplan, `azure.xml`…) plus `mscae/`
  (Microsoft cloud and enterprise).
- `img/lib/azure2/` and `img/lib/mscae/` (Azure icon images).
- `math/es5/startup.js` is an empty stub: MathJax is not bundled, so the viewer never fetches it and LaTeX
  formulas show as text.

Shapes from libraries that are not included (AWS 3/4, Cisco, GCP, Office, rack, Veeam…) render as plain
rectangles with their label. To add one, copy its file from the same draw.io release into the same path.
Before drawing, the viewer strips web fonts (`fontSource`) and remote images from the diagram, and removes
remote references from the SVG it produces.
