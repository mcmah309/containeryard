FROM nvidia/cuda:12.4.1-cudnn-devel-ubuntu22.04

RUN apt-get update && apt-get upgrade -y

RUN apt-get install -y python3 python3-pip python3-venv build-essential python3-dev

RUN ln -s /usr/bin/python3 /usr/bin/python

RUN pip install torch torchvision torchaudio --extra-index-url https://download.pytorch.org/whl/cu124

COPY project/requirements.txt /tmp/requirements.txt

RUN pip install -r /tmp/requirements.txt

WORKDIR /app