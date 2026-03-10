variable "vultr_api_key" {
  description = "Vultr API key (set via TF_VAR_vultr_api_key or terraform.tfvars)"
  type        = string
  sensitive   = true
}

variable "region" {
  description = "Vultr region slug"
  type        = string
  default     = "ewr" # New Jersey
}

variable "plan" {
  description = "Vultr plan slug"
  type        = string
  default     = "vc2-2c-4gb" # 2 vCPU / 4 GB RAM — $20/month
}

variable "os_id" {
  description = "Vultr OS ID (2284 = Ubuntu 24.04 LTS x64)"
  type        = number
  default     = 2284
}

variable "ssh_key_name" {
  description = "Name of the SSH key already uploaded to your Vultr account"
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
  default     = "ghcr.io/anjaustin/mnemo/mnemo-server:latest"
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
  default     = "mnemo_vultr_384_"
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
