# Public invoker for the vsp-parser-poc Cloud Run service (PRS-799).
# Same org-policy exception pattern as protocols-research PR 40: the
# allUsersIngress/allowed tag is bound to this one service, then allUsers may
# invoke it. /v1 stays closed by the service's own bearer token; /health is open.
# The service (built with `gcloud run deploy --source`, see ../backend) and its
# bearer secret were created first and imported; the secret *value* is not in
# Terraform. State is local for now; moving it to a versioned GCS bucket is a
# follow-up.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 6.0"
    }
  }
}

variable "project" {
  type    = string
  default = "anchorage-playground"
}

variable "region" {
  type    = string
  default = "us-central1"
}

variable "service" {
  type    = string
  default = "vsp-parser-poc"
}

# Public key (SEC1 uncompressed P-256 hex, from keygen.mjs) the backend pins to
# enforce proof of payment. Public, so a plain variable; empty disables it.
variable "payment_marker_pubkey" {
  type    = string
  default = ""
}

variable "all_users_ingress_tag_value" {
  type    = string
  default = "tagValues/281480630181594"
}

provider "google" {
  project = var.project
  region  = var.region
}

data "google_project" "current" {
  project_id = var.project
}

resource "google_tags_location_tag_binding" "allow_public_invoker" {
  location  = var.region
  parent    = "//run.googleapis.com/projects/${data.google_project.current.number}/locations/${var.region}/services/${google_cloud_run_v2_service.parser.name}"
  tag_value = var.all_users_ingress_tag_value
}

resource "google_cloud_run_v2_service_iam_member" "public_invoker" {
  name     = google_cloud_run_v2_service.parser.name
  location = var.region
  role     = "roles/run.invoker"
  member   = "allUsers"

  depends_on = [google_tags_location_tag_binding.allow_public_invoker]
}

# Imported: created out-of-band with gcloud. The secret *value* (a version) is
# deliberately not managed here so it never lands in this state.
resource "google_secret_manager_secret" "bearer" {
  secret_id = "vsp-poc-bearer"

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_iam_member" "runtime_reads_bearer" {
  secret_id = google_secret_manager_secret.bearer.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${data.google_project.current.number}-compute@developer.gserviceaccount.com"
}

# Imported: deployed with `gcloud run deploy --source` (image built by Cloud Build).
resource "google_cloud_run_v2_service" "parser" {
  name                = var.service
  location            = var.region
  ingress             = "INGRESS_TRAFFIC_ALL"
  deletion_protection = true

  template {
    scaling {
      max_instance_count = 2
      min_instance_count = 0
    }

    containers {
      image = "us-central1-docker.pkg.dev/anchorage-playground/cloud-run-source-deploy/vsp-parser-poc"

      ports {
        container_port = 8080
      }

      env {
        name  = "GATEWAY_AUTH_BEARER_FILE"
        value = "/secrets/bearer"
      }
      env {
        name  = "PAYMENT_MARKER_PUBKEY"
        value = var.payment_marker_pubkey
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "1Gi"
        }
        startup_cpu_boost = true
        cpu_idle          = true
      }

      volume_mounts {
        name       = "vsp-poc-bearer-goq-geb"
        mount_path = "/secrets"
      }
    }

    volumes {
      name = "vsp-poc-bearer-goq-geb"
      secret {
        secret = google_secret_manager_secret.bearer.secret_id
        items {
          version = "latest"
          path    = "bearer"
        }
      }
    }
  }

  lifecycle {
    # The image is rebuilt by Cloud Build, not Terraform.
    ignore_changes = [build_config, scaling, template[0].containers[0].image, client, client_version, template[0].labels, template[0].revision]
  }
}
