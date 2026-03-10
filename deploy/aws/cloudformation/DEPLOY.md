# Mnemo — AWS CloudFormation Deployment

Deploy the full Mnemo stack on a single EC2 instance with a persistent EBS data volume. All three services (mnemo-server, Redis, Qdrant) run via Docker Compose.

---

## What Gets Created

| Resource | Default | Cost estimate |
|---|---|---|
| EC2 instance | t3.medium (2 vCPU / 4 GB RAM) | ~$30/month |
| EBS root volume | 20 GiB gp3 (OS) | ~$1.60/month |
| EBS data volume | 20 GiB gp3 (Redis + Qdrant) | ~$1.60/month |
| Security Group | ports 8080, 80, 443 open; 22 restricted | Free |
| **Total** | | **~$33/month** |

> The data volume has `DeletionPolicy: Retain` — it survives stack deletion. Delete it manually from the EC2 console when you no longer need the data.

---

## Prerequisites

- AWS account with permissions to create EC2, Security Groups, EBS, and CloudFormation stacks
- An existing EC2 key pair in the target region (create one in EC2 → Key Pairs if needed)
- AWS CLI installed and configured (`aws configure`)

---

## Option A — AWS Console

1. Open **CloudFormation → Stacks → Create stack → With new resources**
2. Select **Upload a template file** → upload `mnemo_cfn.yaml`
3. Fill in parameters:

| Parameter | Required | Notes |
|---|---|---|
| `InstanceType` | No | Default: `t3.medium`. Use `t3.small` for dev/demo only. |
| `DataVolumeSize` | No | Default: 20 GiB. Increase for high write volume. |
| `KeyName` | **Yes** | Must exist in the target region. |
| `SSHCidr` | No | Default: `0.0.0.0/0`. Restrict to your IP in production. |
| `MnemoVersion` | No | Legacy tag parameter. Leave default when using `MnemoImage`. |
| `MnemoImage` | No | Full image reference. Default is the distroless local-embed image used in falsification. |
| `MnemoLlmProvider` | No | Leave blank to skip enrichment. |
| `MnemoLlmApiKey` | No | Your OpenAI/Anthropic key. |
| `MnemoEmbeddingProvider` | No | `local` or remote `openai`-compatible provider. |
| `MnemoEmbeddingDimensions` | No | `384` for `AllMiniLML6V2`; keep aligned with collection prefix. |
| `MnemoQdrantPrefix` | No | Fresh collection namespace for this deployment. |
| `MnemoSessionSummaryThreshold` | No | Progressive summarization threshold; default `10`. |
| `MnemoAuthEnabled` | No | Default: `false`. Set `true` before public exposure. |
| `MnemoAuthApiKeys` | No | Comma-separated keys when auth is enabled. |

4. Click through to **Create stack**. Creation takes ~5 minutes.
5. On the **Outputs** tab, copy the `HealthCheckURL`.

---

## Option B — AWS CLI

```bash
aws cloudformation create-stack \
  --stack-name mnemo \
  --template-body file://mnemo_cfn.yaml \
  --parameters \
    ParameterKey=KeyName,ParameterValue=YOUR_KEY_PAIR_NAME \
    ParameterKey=SSHCidr,ParameterValue=$(curl -s https://checkip.amazonaws.com)/32 \
    ParameterKey=MnemoImage,ParameterValue=ghcr.io/anjaustin/mnemo/mnemo-server:latest \
    ParameterKey=MnemoLlmProvider,ParameterValue=anthropic \
    ParameterKey=MnemoLlmApiKey,ParameterValue=sk-YOUR_KEY \
    ParameterKey=MnemoLlmModel,ParameterValue=claude-haiku-4-20250514 \
    ParameterKey=MnemoEmbeddingProvider,ParameterValue=local \
    ParameterKey=MnemoEmbeddingModel,ParameterValue=AllMiniLML6V2 \
    ParameterKey=MnemoEmbeddingDimensions,ParameterValue=384 \
    ParameterKey=MnemoQdrantPrefix,ParameterValue=mnemo_aws_384_ \
    --region us-east-1

# Watch stack events
aws cloudformation wait stack-create-complete \
  --stack-name mnemo --region us-east-1

# Get outputs
aws cloudformation describe-stacks \
  --stack-name mnemo --region us-east-1 \
  --query "Stacks[0].Outputs" --output table
```

---

## Verify

```bash
# Set IP from outputs
IP=$(aws cloudformation describe-stacks \
  --stack-name mnemo --region us-east-1 \
  --query "Stacks[0].Outputs[?OutputKey=='InstancePublicIP'].OutputValue" \
  --output text)

# Health check
curl http://$IP:8080/health
# Expected: {"status":"ok","version":"0.3.7"}

# Write a memory
curl -s -X POST http://$IP:8080/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"user":"alice","session":"test","text":"Mnemo running on AWS"}'

# Read context
curl -s -X POST http://$IP:8080/api/v1/memory/alice/context \
  -H "Content-Type: application/json" \
  -d '{"query":"AWS","limit":5}'

# Persistence test
aws ec2 reboot-instances --instance-ids \
  $(aws cloudformation describe-stack-resource \
    --stack-name mnemo --logical-resource-id MnemoInstance \
    --query "StackResourceDetail.PhysicalResourceId" --output text)
sleep 60
curl http://$IP:8080/health
# Repeat context query — should return same results
```

---

## SSH Access

```bash
ssh -i ~/.ssh/YOUR_KEY.pem ec2-user@$IP

# Check init log
sudo cat /var/log/mnemo-init.log

# Check stack status
cd /opt/mnemo && docker compose ps
docker compose logs mnemo
```

---

## Updating Mnemo

SSH into the instance and pull the new image:

```bash
cd /opt/mnemo
# Edit .env to set MNEMO_VERSION=0.4.0 (or desired version)
docker compose pull mnemo
docker compose up -d mnemo
curl http://localhost:8080/health
```

---

## Teardown

```bash
# Delete the stack (EC2 + security group are deleted; data volume is RETAINED)
aws cloudformation delete-stack --stack-name mnemo --region us-east-1
aws cloudformation wait stack-delete-complete --stack-name mnemo --region us-east-1

# After confirming data is no longer needed, delete the EBS volume manually:
# EC2 Console → Volumes → filter by tag Name=mnemo-data → Delete
```

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Stack stuck in `CREATE_IN_PROGRESS` > 10 min | SSH in; `sudo cat /var/log/mnemo-init.log` |
| Health check unreachable | Security Group — confirm port 8080 open; check instance is in `running` state |
| `Connection refused` on 8080 | `docker compose ps` — services may still be starting (allow 2–3 min after instance ready) |
| Redis/Qdrant not healthy | `docker compose logs redis qdrant` — check `/data` volume mounted correctly |
| EBS volume not attached | `lsblk` — `/dev/xvdf` should appear; check `MnemoVolumeAttachment` resource in CFN |
| cfn-signal timeout | UserData failed before signaling — check `/var/log/mnemo-init.log` |
