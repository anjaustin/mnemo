terraform {
  required_version = ">= 1.3"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }
}

provider "google" {
  project = var.project
  region  = var.region
  zone    = var.zone
}

# ── Firewall rule ────────────────────────────────────────────────────────────
resource "google_compute_firewall" "mnemo" {
  name    = "mnemo-allow-http-ssh"
  network = "default"

  allow {
    protocol = "tcp"
    ports    = ["22", "80", "443", "8080"]
  }

  source_ranges = ["0.0.0.0/0"]
  target_tags   = ["mnemo-server"]
}

# ── Persistent data disk (Redis + Qdrant) ────────────────────────────────────
resource "google_compute_disk" "mnemo_data" {
  name = "mnemo-data"
  type = "pd-ssd"
  zone = var.zone
  size = var.disk_size_gb

  labels = {
    app = "mnemo"
  }

  lifecycle {
    # Prevent accidental deletion of data disk
    prevent_destroy = false
  }
}

# ── Compute Engine instance ──────────────────────────────────────────────────
resource "google_compute_instance" "mnemo" {
  name         = "mnemo-server"
  machine_type = var.machine_type
  zone         = var.zone

  tags = ["mnemo-server"]

  labels = {
    app = "mnemo"
  }

  boot_disk {
    initialize_params {
      image = "debian-cloud/debian-12"
      size  = 20
      type  = "pd-ssd"
    }
  }

  # Persistent data disk — attached at boot, no race condition
  attached_disk {
    source      = google_compute_disk.mnemo_data.self_link
    device_name = "mnemo-data"
    mode        = "READ_WRITE"
  }

  network_interface {
    network = "default"
    access_config {
      # Ephemeral external IP
    }
  }

  metadata = {
    startup-script = <<-STARTUP
      #!/bin/bash
      exec > /var/log/mnemo-init.log 2>&1
      set -eux

      # ── Mount persistent data disk ──────────────────────────────
      DATA_DEV="/dev/disk/by-id/google-mnemo-data"
      # Wait for device
      for i in $(seq 1 30); do [ -e "$DATA_DEV" ] && break; sleep 2; done

      # Format if new
      if ! blkid "$DATA_DEV"; then mkfs.ext4 "$DATA_DEV"; fi
      mkdir -p /data
      mount "$DATA_DEV" /data
      grep -q "$DATA_DEV" /etc/fstab || echo "$DATA_DEV /data ext4 defaults,nofail 0 2" >> /etc/fstab

      # ── Docker ─────────────────────────────────────────────────
      apt-get update -qq
      apt-get install -y -qq docker.io curl

      systemctl enable --now docker

      # ── Docker Compose plugin ───────────────────────────────────
      mkdir -p /usr/local/lib/docker/cli-plugins
      curl -fsSL "https://github.com/docker/compose/releases/download/v2.24.6/docker-compose-linux-x86_64" \
        -o /usr/local/lib/docker/cli-plugins/docker-compose
      chmod +x /usr/local/lib/docker/cli-plugins/docker-compose

      # ── App directories ─────────────────────────────────────────
      mkdir -p /data/redis /data/qdrant /opt/mnemo

      # ── Environment file ────────────────────────────────────────
      cat > /opt/mnemo/.env <<'EOF'
      MNEMO_VERSION=__MNEMO_VERSION__
      MNEMO_IMAGE=__MNEMO_IMAGE__
      MNEMO_SERVER_PORT=8080
      MNEMO_LLM_PROVIDER=__LLM_PROVIDER__
      MNEMO_LLM_API_KEY=__LLM_API_KEY__
      MNEMO_LLM_MODEL=__LLM_MODEL__
      MNEMO_EMBEDDING_PROVIDER=__EMBED_PROVIDER__
      MNEMO_EMBEDDING_API_KEY=__EMBED_API_KEY__
      MNEMO_EMBEDDING_MODEL=__EMBED_MODEL__
      MNEMO_EMBEDDING_DIMENSIONS=__EMBED_DIMS__
      MNEMO_QDRANT_PREFIX=__QDRANT_PREFIX__
      MNEMO_SESSION_SUMMARY_THRESHOLD=__SESSION_SUMMARY_THRESHOLD__
      MNEMO_AUTH_ENABLED=__AUTH_ENABLED__
      MNEMO_AUTH_API_KEYS=__AUTH_API_KEYS__
      MNEMO_WEBHOOKS_ENABLED=true
      MNEMO_WEBHOOKS_MAX_ATTEMPTS=3
      RUST_LOG=mnemo=info
      EOF

      sed -i \
        -e "s|__MNEMO_VERSION__|${var.mnemo_version}|g" \
        -e "s|__MNEMO_IMAGE__|${var.mnemo_image}|g" \
        -e "s|__LLM_PROVIDER__|${var.mnemo_llm_provider}|g" \
        -e "s|__LLM_API_KEY__|${var.mnemo_llm_api_key}|g" \
        -e "s|__LLM_MODEL__|${var.mnemo_llm_model}|g" \
        -e "s|__EMBED_PROVIDER__|${var.mnemo_embedding_provider}|g" \
        -e "s|__EMBED_API_KEY__|${var.mnemo_embedding_api_key}|g" \
        -e "s|__EMBED_MODEL__|${var.mnemo_embedding_model}|g" \
        -e "s|__EMBED_DIMS__|${var.mnemo_embedding_dimensions}|g" \
        -e "s|__QDRANT_PREFIX__|${var.mnemo_qdrant_prefix}|g" \
        -e "s|__SESSION_SUMMARY_THRESHOLD__|${var.mnemo_session_summary_threshold}|g" \
        -e "s|__AUTH_ENABLED__|${var.mnemo_auth_enabled}|g" \
        -e "s|__AUTH_API_KEYS__|${var.mnemo_auth_api_keys}|g" \
        /opt/mnemo/.env
      chmod 600 /opt/mnemo/.env

      # ── Docker Compose file ─────────────────────────────────────
      MNEMO_IMAGE=$(grep '^MNEMO_IMAGE=' /opt/mnemo/.env | cut -d= -f2-)
      cat > /opt/mnemo/docker-compose.yml <<'EOF'
      services:
        redis:
          image: redis/redis-stack:7.4.0-v1
          container_name: mnemo-redis
          restart: always
          ports:
            - "127.0.0.1:6379:6379"
          volumes:
            - /data/redis:/data
          environment:
            - REDIS_ARGS=--save 60 1 --appendonly yes --loglevel warning
          healthcheck:
            test: ["CMD", "redis-cli", "ping"]
            interval: 10s
            timeout: 5s
            retries: 5
            start_period: 10s
        qdrant:
          image: qdrant/qdrant:v1.12.4
          container_name: mnemo-qdrant
          restart: always
          ports:
            - "127.0.0.1:6334:6334"
          volumes:
            - /data/qdrant:/qdrant/storage
          environment:
            - QDRANT__SERVICE__GRPC_PORT=6334
            - QDRANT__LOG_LEVEL=WARN
          healthcheck:
            test: ["CMD-SHELL", "pidof qdrant || exit 1"]
            interval: 10s
            timeout: 5s
            retries: 5
            start_period: 15s
        mnemo:
          image: __MNEMO_IMAGE__
          container_name: mnemo-server
          restart: always
          ports:
            - "8080:8080"
          env_file:
            - /opt/mnemo/.env
          environment:
            - MNEMO_SERVER_HOST=0.0.0.0
            - MNEMO_SERVER_PORT=8080
            - MNEMO_REDIS_URL=redis://redis:6379
            - MNEMO_QDRANT_URL=http://qdrant:6334
          depends_on:
            redis:
              condition: service_healthy
            qdrant:
              condition: service_healthy
          healthcheck:
            test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
            interval: 15s
            timeout: 5s
            retries: 3
            start_period: 20s
      EOF
      sed -i "s|__MNEMO_IMAGE__|$MNEMO_IMAGE|g" /opt/mnemo/docker-compose.yml

      # ── Start the stack ─────────────────────────────────────────
      cd /opt/mnemo
      docker compose up -d

      echo "Mnemo init complete."
    STARTUP
  }

  service_account {
    scopes = ["cloud-platform"]
  }
}
