export function loadLegacyRendererScript(relativeScriptPath) {
    const scriptUrl = new URL(relativeScriptPath, window.location.href);
    const scriptNode = document.createElement('script');
    scriptNode.src = scriptUrl.toString();
    scriptNode.async = false;
    document.body.appendChild(scriptNode);
}
