terraform {
  required_version = ">= 1.3"
  required_providers {
    linode = {
      source  = "linode/linode"
      version = "~> 2.0"
    }
  }
}

provider "linode" {
  token = var.linode_token
}

# ── Mnemo instance ────────────────────────────────────────────────────────────
resource "linode_instance" "mnemo" {
  label  = "mnemo-server"
  region = var.region
  type   = var.instance_type
  image  = "linode/ubuntu24.04"

  authorized_keys = var.ssh_authorized_keys

  # Boot disk is auto-provisioned from image; extra data disk below
  tags = ["mnemo"]

  metadata {
    user_data = base64encode(templatefile("${path.module}/startup.sh.tpl", {
      mnemo_version          = var.mnemo_version
      mnemo_llm_provider     = var.mnemo_llm_provider
      mnemo_llm_api_key      = var.mnemo_llm_api_key
      mnemo_llm_model        = var.mnemo_llm_model
      mnemo_embedding_api_key = var.mnemo_embedding_api_key
      mnemo_embedding_model  = var.mnemo_embedding_model
      mnemo_auth_enabled     = var.mnemo_auth_enabled
      mnemo_auth_api_keys    = var.mnemo_auth_api_keys
    }))
  }
}

# ── Firewall ──────────────────────────────────────────────────────────────────
resource "linode_firewall" "mnemo" {
  label = "mnemo-firewall"

  inbound_policy  = "DROP"
  outbound_policy = "ACCEPT"

  inbound {
    label    = "allow-ssh"
    action   = "ACCEPT"
    protocol = "TCP"
    ports    = "22"
    ipv4     = ["0.0.0.0/0"]
    ipv6     = ["::/0"]
  }

  inbound {
    label    = "allow-mnemo"
    action   = "ACCEPT"
    protocol = "TCP"
    ports    = "8080"
    ipv4     = ["0.0.0.0/0"]
    ipv6     = ["::/0"]
  }

  inbound {
    label    = "allow-http-https"
    action   = "ACCEPT"
    protocol = "TCP"
    ports    = "80,443"
    ipv4     = ["0.0.0.0/0"]
    ipv6     = ["::/0"]
  }

  linodes = [linode_instance.mnemo.id]
}
