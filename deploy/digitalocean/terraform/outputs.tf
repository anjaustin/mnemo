output "droplet_ip" {
  description = "Public IPv4 address of the Mnemo Droplet"
  value       = digitalocean_droplet.mnemo.ipv4_address
}

output "health_check_url" {
  description = "Health check URL"
  value       = "http://${digitalocean_droplet.mnemo.ipv4_address}:8080/health"
}

output "ssh_command" {
  description = "SSH command to access the Droplet"
  value       = "ssh root@${digitalocean_droplet.mnemo.ipv4_address}"
}

output "init_log_command" {
  description = "Command to tail the init log"
  value       = "ssh root@${digitalocean_droplet.mnemo.ipv4_address} 'tail -f /var/log/mnemo-init.log'"
}
