variable "linode_token" {
  description = "Linode API token (set via TF_VAR_linode_token or terraform.tfvars)"
  type        = string
  sensitive   = true
}

variable "region" {
  description = "Linode region"
  type        = string
  default     = "us-ord"
}

variable "instance_type" {
  description = "Linode instance type"
  type        = string
  default     = "g6-standard-2"  # 2 vCPU / 4 GB RAM — ~$18/month
}

variable "ssh_authorized_keys" {
  description = "List of SSH public keys to authorize on the instance"
  type        = list(string)
  default     = []
}

variable "mnemo_version" {
  description = "Mnemo server image tag"
  type        = string
  default     = "latest"
}

variable "mnemo_llm_provider" {
  description = "LLM provider (openai | anthropic | ollama | liquid)"
  type        = string
  default     = "openai"
}

variable "mnemo_llm_api_key" {
  description = "LLM API key"
  type        = string
  default     = ""
  sensitive   = true
}

variable "mnemo_llm_model" {
  description = "LLM model name"
  type        = string
  default     = "gpt-4o-mini"
}

variable "mnemo_embedding_api_key" {
  description = "Embedding API key (defaults to LLM key if same provider)"
  type        = string
  default     = ""
  sensitive   = true
}

variable "mnemo_embedding_model" {
  description = "Embedding model name"
  type        = string
  default     = "text-embedding-3-small"
}

variable "mnemo_auth_enabled" {
  description = "Enable API key auth (true/false)"
  type        = string
  default     = "false"
}

variable "mnemo_auth_api_keys" {
  description = "Comma-separated API keys (required when auth_enabled=true)"
  type        = string
  default     = ""
  sensitive   = true
}
