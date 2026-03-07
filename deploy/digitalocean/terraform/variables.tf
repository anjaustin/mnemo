variable "do_token" {
  description = "DigitalOcean API token (set via TF_VAR_do_token or terraform.tfvars)"
  type        = string
  sensitive   = true
}

variable "region" {
  description = "DigitalOcean region slug"
  type        = string
  default     = "nyc3"
}

variable "droplet_size" {
  description = "Droplet size slug"
  type        = string
  default     = "s-2vcpu-4gb" # 2 vCPU / 4 GB RAM — ~$24/month
}

variable "ssh_key_name" {
  description = "Name of the SSH key already uploaded to your DigitalOcean account"
  type        = string
}

variable "mnemo_version" {
  description = "Mnemo server image tag"
  type        = string
  default     = "latest"
}

variable "mnemo_image" {
  description = "Full Mnemo server image reference"
  type        = string
  default     = "ttl.sh/mnemo-local-embed-distroless-fixed-20260307:24h"
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
  description = "Embedding API key"
  type        = string
  default     = ""
  sensitive   = true
}

variable "mnemo_embedding_model" {
  description = "Embedding model name"
  type        = string
  default     = "text-embedding-3-small"
}

variable "mnemo_embedding_provider" {
  description = "Embedding provider (openai | local)"
  type        = string
  default     = "local"
}

variable "mnemo_embedding_dimensions" {
  description = "Embedding vector dimensions"
  type        = string
  default     = "384"
}

variable "mnemo_qdrant_prefix" {
  description = "Qdrant collection prefix"
  type        = string
  default     = "mnemo_do_384_"
}

variable "mnemo_session_summary_threshold" {
  description = "Episode threshold for progressive session summarization"
  type        = string
  default     = "10"
}

variable "mnemo_auth_enabled" {
  description = "Enable API key auth (true/false)"
  type        = string
  default     = "false"
}

variable "mnemo_auth_api_keys" {
  description = "Comma-separated API keys"
  type        = string
  default     = ""
  sensitive   = true
}
