use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

pub fn merge_resource_meta(mut base: ObjectMeta, extra: &Option<ObjectMeta>) -> ObjectMeta {
    if let Some(extra) = extra {
        if let Some(labels) = &extra.labels {
            base.labels
                .get_or_insert_with(Default::default)
                .extend(labels.clone());
        }

        if let Some(annotations) = &extra.annotations {
            base.annotations
                .get_or_insert_with(Default::default)
                .extend(annotations.clone());
        }
    }
    base
}
