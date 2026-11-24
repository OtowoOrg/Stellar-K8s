import React, {useState} from 'react';

function yamlEscape(s){
  return String(s).replace(/\r\n/g,'\n');
}

export default function ArgocdGenerator(){
  const [appName, setAppName] = useState('stellar-validator');
  const [namespace, setNamespace] = useState('stellar');
  const [repo, setRepo] = useState('https://github.com/agnesnaomiolim-cloud/Stellar-K8s');
  const [path, setPath] = useState('charts/stellar-operator');
  const [revision, setRevision] = useState('HEAD');
  const [values, setValues] = useState('# Paste Helm values here\n');

  const generate = () => {
    const app = `apiVersion: argoproj.io/v1alpha1\nkind: Application\nmetadata:\n  name: ${appName}\n  namespace: ${namespace}\nspec:\n  project: default\n  destination:\n    server: https://kubernetes.default.svc\n    namespace: ${namespace}\n  source:\n    repoURL: ${repo}\n    path: ${path}\n    targetRevision: ${revision}\n    helm:\n      valueFiles: []\n      values: |\n${yamlEscape(values).split('\n').map(l=> '        '+l).join('\n')}\n  syncPolicy:\n    automated:\n      prune: true\n      selfHeal: true\n`;

    return app;
  };

  const output = generate();

  const download = () => {
    const blob = new Blob([output], {type: 'text/yaml'});
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${appName}-argocd-application.yaml`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  };

  return (
    <div style={{border:'1px solid #e1e1e1',padding:16,borderRadius:6}}>
      <h3>ArgoCD Application Generator</h3>
      <div style={{display:'grid',gridTemplateColumns:'1fr 1fr',gap:8}}>
        <label>App name<input value={appName} onChange={e=>setAppName(e.target.value)} /></label>
        <label>Namespace<input value={namespace} onChange={e=>setNamespace(e.target.value)} /></label>
        <label>Repo URL<input value={repo} onChange={e=>setRepo(e.target.value)} /></label>
        <label>Path<input value={path} onChange={e=>setPath(e.target.value)} /></label>
        <label>Revision<input value={revision} onChange={e=>setRevision(e.target.value)} /></label>
      </div>
      <div style={{marginTop:12}}>
        <label>Helm values (optional)</label>
        <textarea rows={8} style={{width:'100%'}} value={values} onChange={e=>setValues(e.target.value)} />
      </div>
      <div style={{display:'flex',gap:8,marginTop:12}}>
        <button onClick={download}>Download YAML</button>
        <button onClick={()=>{navigator.clipboard && navigator.clipboard.writeText(output)}}>Copy to clipboard</button>
      </div>
      <div style={{marginTop:12}}>
        <label>Generated Application YAML</label>
        <pre style={{background:'#fafafa',padding:12,overflowX:'auto'}}>{output}</pre>
      </div>
    </div>
  );
}
