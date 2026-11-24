# Custom Resource Definition (CRD) Architecture Reference Manual

main
|---|---|---|
| Type | string | `.spec.nodeType` |
| Network | string | `.spec.network` |
| Ready | string | `.status.conditions[?(@.type=='Ready')].status` |
| Replicas | integer | `.spec.replicas` |
| Age | date | `.metadata.creationTimestamp` |

