#!/bin/bash
source /mnt/data/anaconda3/etc/profile.d/conda.sh
conda activate /mnt/data/anaconda3/envs/openai
#/usr/sbin/nginx &&
cd /mnt/data/GPT_API
uvicorn main:app --host 0.0.0.0 --port 7000 --proxy-headers --workers 16
