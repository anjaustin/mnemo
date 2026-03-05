output "instance_ip" {
  description = "Public IPv4 address of the Mnemo instance"
  value       = linode_instance.mnemo.ip_address
}

output "health_check_url" {
  description = "Health check URL"
  value       = "http://${linode_instance.mnemo.ip_address}:8080/health"
}

output "ssh_command" {
  description = "SSH command to access the instance"
  value       = "ssh root@${linode_instance.mnemo.ip_address}"
}

output "init_log_command" {
  description = "Command to tail the init log"
  value       = "ssh root@${linode_instance.mnemo.ip_address} 'tail -f /var/log/mnemo-init.log'"
}
