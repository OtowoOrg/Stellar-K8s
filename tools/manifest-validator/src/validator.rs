use serde_yaml::Value;

fn get_path<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for p in path {
        cur = match cur.get(p) {
            Some(next) => next,
            None => return None,
        }
    }
    Some(cur)
}

pub fn validate_document(v: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    // Basic required fields
    if !v.is_mapping() {
        errors.push("document is not a mapping/object".to_string());
        return errors;
    }

    if get_path(v, &["apiVersion"]).is_none() {
        errors.push("missing 'apiVersion'".to_string());
    }
    if get_path(v, &["kind"]).is_none() {
        errors.push("missing 'kind'".to_string());
    }
    if get_path(v, &["metadata"]).is_none() {
        errors.push("missing 'metadata'".to_string());
    } else if get_path(v, &["metadata", "name"]).is_none() {
        errors.push("missing 'metadata.name'".to_string());
    }

    // Resource limits checks for common pod templates
    if let Some(kind_v) = get_path(v, &["kind"]) {
        if let Some(kind) = kind_v.as_str() {
            let containers_paths: Vec<Vec<&str>> = match kind {
                "Pod" => vec![vec!["spec", "containers"]],
                "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" => vec![
                    vec!["spec", "template", "spec", "containers"],
                ],
                _ => vec![],
            };

            for cp in containers_paths {
                if let Some(containers_v) = get_path(v, &cp) {
                    if let Some(containers) = containers_v.as_sequence() {
                        for (idx, c) in containers.iter().enumerate() {
                            if let Some(name) = get_path(c, &["name"]).and_then(|n| n.as_str()) {
                                let res = get_path(c, &["resources", "limits"]);
                                if res.is_none() {
                                    errors.push(format!(
                                        "container '{}' missing resources.limits",
                                        name
                                    ));
                                } else {
                                    let limits = res.unwrap();
                                    if limits.get("cpu").is_none() {
                                        errors.push(format!(
                                            "container '{}' missing resources.limits.cpu",
                                            name
                                        ));
                                    }
                                    if limits.get("memory").is_none() {
                                        errors.push(format!(
                                            "container '{}' missing resources.limits.memory",
                                            name
                                        ));
                                    }
                                }
                            } else {
                                errors.push(format!(
                                    "container[{}] missing name field",
                                    idx
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    // Network checks: hostNetwork and hostPort usage
    if let Some(spec) = get_path(v, &["spec"]) {
        if let Some(host_net) = spec.get("hostNetwork") {
            if host_net.as_bool().unwrap_or(false) {
                errors.push("spec.hostNetwork is true (avoid hostNetwork when possible)".to_string());
            }
        }
    }

    // Check for hostPort in container ports
    if let Some(kind_v) = get_path(v, &["kind"]) {
        if let Some(kind) = kind_v.as_str() {
            let containers_paths: Vec<Vec<&str>> = match kind {
                "Pod" => vec![vec!["spec", "containers"]],
                "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" => vec![
                    vec!["spec", "template", "spec", "containers"],
                ],
                _ => vec![],
            };

            for cp in containers_paths {
                if let Some(containers_v) = get_path(v, &cp) {
                    if let Some(containers) = containers_v.as_sequence() {
                        for c in containers.iter() {
                            if let Some(ports) = get_path(c, &["ports"]).and_then(|p| p.as_sequence()) {
                                for p in ports {
                                    if p.get("hostPort").is_some() {
                                        errors.push("container uses hostPort (consider avoiding hostPort)".to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Secret reference checks: volumes and env secretKeyRef
    // Volumes
    if let Some(vols) = get_path(v, &["spec", "volumes"]).and_then(|v| v.as_sequence()) {
        for vol in vols {
            if vol.get("secret").is_some() {
                if vol.get("secret").and_then(|s| s.get("secretName")).is_none() {
                    errors.push("volume.secret is missing secretName".to_string());
                }
            }
        }
    }

    // env/envFrom in containers
    // Search common container locations
    let container_common_paths = vec![vec!["spec", "containers"], vec!["spec", "template", "spec", "containers"]];
    for cp in container_common_paths {
        if let Some(containers_v) = get_path(v, &cp) {
            if let Some(containers) = containers_v.as_sequence() {
                for c in containers {
                    if let Some(env) = c.get("env").and_then(|e| e.as_sequence()) {
                        for ev in env {
                            if let Some(vf) = ev.get("valueFrom") {
                                if vf.get("secretKeyRef").is_some() {
                                    let sk = vf.get("secretKeyRef").unwrap();
                                    if sk.get("name").is_none() {
                                        errors.push("env.valueFrom.secretKeyRef missing name".to_string());
                                    }
                                }
                            }
                        }
                    }
                    if let Some(envfrom) = c.get("envFrom").and_then(|e| e.as_sequence()) {
                        for ef in envfrom {
                            if ef.get("secretRef").is_some() {
                                let sr = ef.get("secretRef").unwrap();
                                if sr.get("name").is_none() {
                                    errors.push("envFrom.secretRef missing name".to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Storage class checks for PersistentVolumeClaims
    if let Some(kind_v) = get_path(v, &["kind"]) {
        if let Some(kind) = kind_v.as_str() {
            if kind == "PersistentVolumeClaim" || kind == "PersistentVolumeClaim" {
                if let Some(sc) = get_path(v, &["spec", "storageClassName"]) {
                    if sc.is_null() {
                        errors.push("PersistentVolumeClaim.spec.storageClassName is null or missing".to_string());
                    }
                } else {
                    errors.push("PersistentVolumeClaim missing spec.storageClassName".to_string());
                }
                // check requests.storage
                if let Some(req) = get_path(v, &["spec", "resources", "requests", "storage"]) {
                    if req.as_str().unwrap_or("").is_empty() {
                        errors.push("PersistentVolumeClaim.spec.resources.requests.storage is empty".to_string());
                    }
                } else {
                    errors.push("PersistentVolumeClaim missing spec.resources.requests.storage".to_string());
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_minimal_pod() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: test-pod
spec:
  containers:
    - name: c1
      image: busybox
"#;

        let v: Value = serde_yaml::from_str(yaml).unwrap();
        let errs = validate_document(&v);
        assert!(errs.iter().any(|e| e.contains("resources.limits")));
    }
}
