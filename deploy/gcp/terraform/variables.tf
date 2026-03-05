variable "project" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "GCP zone"
  type        = string
  default     = "us-central1-a"
}

variable "machine_type" {
  description = "Compute Engine machine type. e2-medium (2 vCPU / 4 GB) is the recommended minimum."
  type        = string
  default     = "e2-medium"
}

variable "disk_size_gb" {
  description = "Size in GB for the persistent data disk (Redis + Qdrant)."
  type        = number
  default     = 20
}

variable "mnemo_version" {
  description = "mnemo-server Docker image tag. Only 'latest' is currently published to GHCR."
  type        = string
  default     = "latest"
}

variable "mnemo_llm_provider" {
  description = "LLM provider: openai | anthropic | ollama | liquid (leave blank to skip enrichment)"
  type        = string
  default     = ""
}

variable "mnemo_llm_api_key" {
  description = "API key for the LLM provider"
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
  description = "API key for the embedding model"
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
  description = "Enable API key authentication. Set to true before public exposure."
  type        = string
  default     = "false"
}

variable "mnemo_auth_api_keys" {
  description = "Comma-separated API keys (required when mnemo_auth_enabled=true)"
  type        = string
  default     = ""
  sensitive   = true
}
