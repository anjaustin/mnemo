terraform {
  required_version = ">= 1.3"
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "~> 2.0"
    }
  }
}

provider "digitalocean" {
  token = var.do_token
}

# ── SSH key lookup (must already exist in DO account) ─────────────────────────
data "digitalocean_ssh_key" "mnemo" {
  name = var.ssh_key_name
}

# ── Mnemo Droplet ─────────────────────────────────────────────────────────────
resource "digitalocean_droplet" "mnemo" {
  name   = "mnemo-server"
  region = var.region
  size   = var.droplet_size
  image  = "ubuntu-24-04-x64"

  ssh_keys  = [data.digitalocean_ssh_key.mnemo.id]
  user_data = templatefile("${path.module}/startup.sh.tpl", {
    mnemo_version           = var.mnemo_version
    mnemo_llm_provider      = var.mnemo_llm_provider
    mnemo_llm_api_key       = var.mnemo_llm_api_key
    mnemo_llm_model         = var.mnemo_llm_model
    mnemo_embedding_api_key = var.mnemo_embedding_api_key
    mnemo_embedding_model   = var.mnemo_embedding_model
    mnemo_auth_enabled      = var.mnemo_auth_enabled
    mnemo_auth_api_keys     = var.mnemo_auth_api_keys
  })

  tags = ["mnemo"]
}

# ── Firewall ──────────────────────────────────────────────────────────────────
resource "digitalocean_firewall" "mnemo" {
  name = "mnemo-firewall"

  droplet_ids = [digitalocean_droplet.mnemo.id]

  inbound_rule {
    protocol         = "tcp"
    port_range       = "22"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  inbound_rule {
    protocol         = "tcp"
    port_range       = "8080"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  inbound_rule {
    protocol         = "tcp"
    port_range       = "80"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  inbound_rule {
    protocol         = "tcp"
    port_range       = "443"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "tcp"
    port_range            = "1-65535"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "udp"
    port_range            = "1-65535"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "icmp"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}
