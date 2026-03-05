output "instance_ip" {
  description = "Public IPv4 address of the Mnemo instance"
  value       = vultr_instance.mnemo.main_ip
}

output "health_check_url" {
  description = "Health check URL"
  value       = "http://${vultr_instance.mnemo.main_ip}:8080/health"
}

output "ssh_command" {
  description = "SSH command to access the instance"
  value       = "ssh root@${vultr_instance.mnemo.main_ip}"
}

output "init_log_command" {
  description = "Command to tail the init log"
  value       = "ssh root@${vultr_instance.mnemo.main_ip} 'tail -f /var/log/mnemo-init.log'"
}
