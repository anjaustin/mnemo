terraform {
  required_version = ">= 1.3"
  required_providers {
    vultr = {
      source  = "vultr/vultr"
      version = "~> 2.0"
    }
  }
}

provider "vultr" {
  api_key = var.vultr_api_key
}

# ── SSH key lookup (must already exist in Vultr account) ──────────────────────
data "vultr_ssh_key" "mnemo" {
  filter {
    name   = "name"
    values = [var.ssh_key_name]
  }
}

# ── Mnemo Instance ────────────────────────────────────────────────────────────
resource "vultr_instance" "mnemo" {
  plan     = var.plan
  region   = var.region
  os_id    = var.os_id
  label    = "mnemo-server"
  hostname = "mnemo-vultr"
  tags     = ["mnemo"]

  ssh_key_ids = [data.vultr_ssh_key.mnemo.id]
  user_data = templatefile("${path.module}/startup.sh.tpl", {
    mnemo_version                   = var.mnemo_version
    mnemo_image                     = var.mnemo_image
    mnemo_llm_provider              = var.mnemo_llm_provider
    mnemo_llm_api_key               = var.mnemo_llm_api_key
    mnemo_llm_model                 = var.mnemo_llm_model
    mnemo_embedding_provider        = var.mnemo_embedding_provider
    mnemo_embedding_api_key         = var.mnemo_embedding_api_key
    mnemo_embedding_model           = var.mnemo_embedding_model
    mnemo_embedding_dimensions      = var.mnemo_embedding_dimensions
    mnemo_qdrant_prefix             = var.mnemo_qdrant_prefix
    mnemo_session_summary_threshold = var.mnemo_session_summary_threshold
    mnemo_auth_enabled              = var.mnemo_auth_enabled
    mnemo_auth_api_keys             = var.mnemo_auth_api_keys
  })
  activation_email = false
}

# ── Firewall ──────────────────────────────────────────────────────────────────
resource "vultr_firewall_group" "mnemo" {
  description = "mnemo-firewall"
}

resource "vultr_firewall_rule" "ssh" {
  firewall_group_id = vultr_firewall_group.mnemo.id
  protocol          = "tcp"
  ip_type           = "v4"
  subnet            = "0.0.0.0"
  subnet_size       = 0
  port              = "22"
  notes             = "SSH"
}

resource "vultr_firewall_rule" "http" {
  firewall_group_id = vultr_firewall_group.mnemo.id
  protocol          = "tcp"
  ip_type           = "v4"
  subnet            = "0.0.0.0"
  subnet_size       = 0
  port              = "8080"
  notes             = "Mnemo API"
}

resource "vultr_firewall_rule" "https" {
  firewall_group_id = vultr_firewall_group.mnemo.id
  protocol          = "tcp"
  ip_type           = "v4"
  subnet            = "0.0.0.0"
  subnet_size       = 0
  port              = "443"
  notes             = "HTTPS"
}
