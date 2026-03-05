output "instance_external_ip" {
  description = "External IP address of the Mnemo instance"
  value       = google_compute_instance.mnemo.network_interface[0].access_config[0].nat_ip
}

output "health_check_url" {
  description = "Mnemo health check endpoint"
  value       = "http://${google_compute_instance.mnemo.network_interface[0].access_config[0].nat_ip}:8080/health"
}

output "ssh_command" {
  description = "SSH command to access the instance"
  value       = "gcloud compute ssh ${google_compute_instance.mnemo.name} --zone=${var.zone} --project=${var.project}"
}

output "init_log_command" {
  description = "Command to tail the UserData init log"
  value       = "gcloud compute ssh ${google_compute_instance.mnemo.name} --zone=${var.zone} --project=${var.project} -- 'sudo tail -f /var/log/mnemo-init.log'"
}
